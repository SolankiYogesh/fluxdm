//! FTP downloading over a single passive-mode connection, resuming with REST.

use crate::http::{backoff, ensure_destination, fatal, Fatal, JobContext, Shared};
use crate::segments::SegmentMap;
use crate::types::DownloadRequest;
use anyhow::{anyhow, Context, Result};
use percent_encoding::percent_decode_str;
use std::fs::OpenOptions;
use std::time::Duration;
use suppaftp::tokio::AsyncFtpStream;
use suppaftp::types::FileType;
use suppaftp::Mode;
use tokio::io::AsyncReadExt;

const TIMEOUT: Duration = Duration::from_secs(45);

/// Percent-decode a URL part, rejecting line breaks that would smuggle extra
/// FTP commands.
fn decode(part: &str) -> Result<String> {
    let text = percent_decode_str(part)
        .decode_utf8()
        .map_err(|_| fatal("FTP address is not valid UTF-8"))?
        .into_owned();
    if text.contains(['\r', '\n', '\0']) {
        return Err(fatal("FTP address contains forbidden characters"));
    }
    Ok(text)
}

async fn within<T, E: std::fmt::Display>(
    what: &str,
    f: impl std::future::Future<Output = Result<T, E>>,
) -> Result<T> {
    tokio::time::timeout(TIMEOUT, f)
        .await
        .map_err(|_| anyhow!("FTP {what} timed out"))?
        .map_err(|e| anyhow!("FTP {what}: {e}"))
}

/// Connect, sign in and return the connection with the file's path.
async fn open(req: &DownloadRequest) -> Result<(AsyncFtpStream, String)> {
    let url = url::Url::parse(&req.url)?;
    let host = url.host_str().ok_or_else(|| fatal("FTP address has no host"))?.to_string();
    let host = host.trim_matches(['[', ']']).to_string();
    let port = url.port().unwrap_or(21);
    let user = match (&req.username, url.username()) {
        (Some(u), _) if !u.is_empty() => u.clone(),
        (_, "") => "anonymous".into(),
        (_, u) => decode(u)?,
    };
    let password = match (&req.password, url.password()) {
        (Some(p), _) if !p.is_empty() => p.clone(),
        (_, Some(p)) => decode(p)?,
        _ => "anonymous@".into(),
    };
    let path = decode(url.path().trim_start_matches('/'))?;

    let mut ftp = within("connect", AsyncFtpStream::connect((host.as_str(), port))).await?;
    ftp.set_mode(Mode::Passive);
    ftp.set_passive_nat_workaround(true);
    within("login", ftp.login(user.as_str(), password.as_str())).await?;
    within("binary mode", ftp.transfer_type(FileType::Binary)).await?;
    Ok((ftp, path))
}

/// The file's size, if the server will say.
pub(crate) async fn size_of(req: &DownloadRequest) -> Result<Option<u64>> {
    let (mut ftp, path) = open(req).await?;
    let size = within("size", ftp.size(path.as_str())).await.ok().map(|n| n as u64);
    let _ = tokio::time::timeout(Duration::from_secs(2), ftp.quit()).await;
    Ok(size)
}

async fn attempt(ctx: &JobContext, task: &Shared) -> Result<()> {
    let req = task.lock().unwrap().req.clone();
    let (mut ftp, path) = open(&req).await?;
    let total = within("size", ftp.size(path.as_str())).await.ok().map(|n| n as u64);

    let (file, offset) = {
        let mut t = task.lock().unwrap();
        ensure_destination(&ctx.settings, &mut t)?;
        let part = t.part_path().expect("destination set");
        t.segments.reset_runtime();
        let saved = t.segments.segs.first().map_or(0, |s| s.done);
        let on_disk = std::fs::metadata(&part).map_or(0, |m| m.len());
        let resume = saved > 0 && t.total.is_some() && t.total == total && on_disk >= saved;
        let offset = if resume { saved } else { 0 };
        if !resume {
            t.segments = SegmentMap::new(total);
            t.total = total;
        }
        t.resumable = Some(total.is_some());
        let file = OpenOptions::new().write(true).create(true).truncate(false).open(&part)?;
        file.set_len(offset)?;
        t.segments.take(0);
        (file, offset)
    };

    if total != Some(offset) {
        if offset > 0 {
            within("resume", ftp.resume_transfer(offset as usize)).await?;
        }
        let mut stream = within("retrieve", ftp.retr_as_stream(path.as_str())).await?;
        let mut buf = vec![0u8; 64 * 1024];
        let mut last_save = std::time::Instant::now();
        loop {
            let n = within("read", stream.read(&mut buf)).await?;
            if n == 0 {
                break;
            }
            let reserved = task.lock().unwrap().segments.reserve(0, n);
            let Some((at, n)) = reserved else { break };
            ctx.global_limit.acquire(n).await;
            ctx.task_limit.acquire(n).await;
            crate::fsio::write_at(&file, &buf[..n], at)?;
            task.lock().unwrap().segments.commit(0, n);
            if last_save.elapsed() > Duration::from_secs(2) {
                last_save = std::time::Instant::now();
                file.sync_data()?;
                let snapshot = task.lock().unwrap().clone();
                (ctx.save)(&snapshot);
            }
        }
        within("finish", stream.finish()).await?;
    }
    let _ = tokio::time::timeout(Duration::from_secs(5), ftp.quit()).await;
    file.sync_all()?;

    let mut t = task.lock().unwrap();
    t.segments.release(0);
    if t.total.is_none() {
        t.segments.seal(0);
        t.total = Some(t.segments.downloaded());
    }
    if !t.segments.is_complete() {
        anyhow::bail!("connection closed early");
    }
    let part = t.part_path().expect("destination set");
    let dir = t.dir.clone().context("no folder")?;
    let target = crate::fsio::move_unique(&part, &dir, &t.file_name)?;
    t.file_name = target.file_name().context("file name")?.to_string_lossy().into_owned();
    Ok(())
}

pub async fn download(ctx: JobContext, task: Shared) -> Result<()> {
    let mut failures = 0;
    loop {
        let before = task.lock().unwrap().segments.downloaded();
        match attempt(&ctx, &task).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if e.downcast_ref::<Fatal>().is_some() || e.downcast_ref::<std::io::Error>().is_some() {
                    return Err(e);
                }
                let progressed = task.lock().unwrap().segments.downloaded() > before;
                failures = if progressed { 1 } else { failures + 1 };
                if failures > ctx.settings.max_retries {
                    return Err(e);
                }
                tokio::time::sleep(backoff(failures)).await;
            }
        }
    }
}
