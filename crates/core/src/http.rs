//! Segmented HTTP(S) downloading.

use crate::limiter::Limiter;
use crate::segments::SegmentMap;
use crate::types::{DownloadRequest, Settings, Task, MAX_CONNECTIONS};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Method, Response, StatusCode};
use std::fs::{File, OpenOptions};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::task::JoinSet;

const IDLE_TIMEOUT: Duration = Duration::from_secs(45);
const CHECKPOINT_EVERY: Duration = Duration::from_secs(2);

pub type Shared = Arc<Mutex<Task>>;
/// Called with a consistent snapshot once its bytes are flushed to disk.
pub type SaveFn = Arc<dyn Fn(&Task) + Send + Sync>;

#[derive(Clone)]
pub struct JobContext {
    pub settings: Settings,
    pub global_limit: Arc<Limiter>,
    pub task_limit: Arc<Limiter>,
    pub save: SaveFn,
}

pub fn build_client(settings: &Settings) -> Result<Client> {
    // No automatic decompression: byte ranges must refer to the stored bytes.
    let mut b = Client::builder()
        .connect_timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::limited(15))
        // One TCP connection per segment; HTTP/2 would multiplex them onto one.
        .http1_only();
    if !settings.proxy.trim().is_empty() {
        b = b.proxy(reqwest::Proxy::all(settings.proxy.trim()).context("invalid proxy address")?);
    }
    b.build().context("create HTTP client")
}

/// A failure that retrying cannot fix.
#[derive(Debug)]
pub struct Fatal(pub String);
impl std::fmt::Display for Fatal {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Fatal {}

pub fn fatal(msg: impl Into<String>) -> anyhow::Error {
    anyhow::Error::new(Fatal(msg.into()))
}

#[derive(Debug)]
pub struct BadStatus(pub StatusCode);
impl std::fmt::Display for BadStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "server replied {}", self.0)
    }
}
impl std::error::Error for BadStatus {}

fn is_permanent(e: &anyhow::Error) -> bool {
    if e.downcast_ref::<Fatal>().is_some() || e.downcast_ref::<std::io::Error>().is_some() {
        return true;
    }
    matches!(e.downcast_ref::<BadStatus>(), Some(BadStatus(s))
        if s.is_client_error() && *s != StatusCode::TOO_MANY_REQUESTS && *s != StatusCode::REQUEST_TIMEOUT)
}

pub fn backoff(attempt: u32) -> Duration {
    Duration::from_millis(500u64.saturating_mul(1 << attempt.min(6)).min(30_000))
}

fn same_origin(a: &str, b: &str) -> bool {
    match (url::Url::parse(a), url::Url::parse(b)) {
        (Ok(a), Ok(b)) => a.origin() == b.origin(),
        _ => false,
    }
}

/// Send a request carrying the browser context. `url` may differ from the
/// original (redirect target or playlist entry); credentials are only attached
/// when it shares the original origin.
pub async fn send(
    client: &Client,
    settings: &Settings,
    req: &DownloadRequest,
    url: &str,
    extra: &HeaderMap,
) -> Result<Response> {
    let trusted = same_origin(&req.url, url);
    let build = || {
        let mut r = client
            .request(Method::GET, url)
            .header(header::ACCEPT, "*/*")
            .header(header::ACCEPT_ENCODING, "identity")
            .header(header::USER_AGENT, req.user_agent.as_deref().unwrap_or(&settings.user_agent))
            .headers(extra.clone());
        if let Some(v) = &req.referrer {
            r = r.header(header::REFERER, v);
        }
        if trusted {
            if let Some(v) = &req.cookie {
                r = r.header(header::COOKIE, v);
            }
        }
        r
    };
    let timed = |r: reqwest::RequestBuilder| async move {
        tokio::time::timeout(IDLE_TIMEOUT, r.send())
            .await
            .map_err(|_| anyhow!("server did not answer in time"))?
            .map_err(|e| anyhow!(e.without_url()))
    };
    let resp = timed(build()).await?;
    let user = req.username.as_deref().filter(|u| !u.is_empty());
    let (Some(user), true, true) = (user, trusted, resp.status() == StatusCode::UNAUTHORIZED) else {
        return Ok(resp);
    };
    let password = req.password.as_deref().unwrap_or("");
    let challenges: Vec<&str> = resp
        .headers()
        .get_all(header::WWW_AUTHENTICATE)
        .iter()
        .filter_map(|v| v.to_str().ok())
        .collect();
    if let Some(digest) = challenges.iter().find(|c| c.trim_start().starts_with("Digest")) {
        let parsed = url::Url::parse(url)?;
        let uri = match parsed.query() {
            Some(q) => format!("{}?{q}", parsed.path()),
            None => parsed.path().to_string(),
        };
        let mut prompt = digest_auth::parse(digest).map_err(|e| fatal(e.to_string()))?;
        let ctx = digest_auth::AuthContext::new(user, password, uri);
        let answer = prompt.respond(&ctx).map_err(|e| fatal(e.to_string()))?;
        timed(build().header(header::AUTHORIZATION, answer.to_header_string())).await
    } else {
        timed(build().basic_auth(user, Some(password))).await
    }
}

fn header_str(h: &HeaderMap, name: header::HeaderName) -> Option<String> {
    h.get(name).and_then(|v| v.to_str().ok()).map(str::to_string)
}

/// `bytes a-b/total` → `(a, total)`.
fn parse_content_range(v: &str) -> Option<(u64, u64)> {
    let rest = v.trim().strip_prefix("bytes")?.trim_start();
    let (range, total) = rest.split_once('/')?;
    let (start, _) = range.split_once('-')?;
    Some((start.trim().parse().ok()?, total.trim().parse().ok()?))
}

/// A matching or differing ETag decides; the date only matters without one,
/// because mirrors often disagree about it.
fn validator_changed(
    etag: &Option<String>,
    new_etag: &Option<String>,
    modified: &Option<String>,
    new_modified: &Option<String>,
) -> bool {
    match (etag, new_etag) {
        (Some(a), Some(b)) => a != b,
        _ => matches!((modified, new_modified), (Some(a), Some(b)) if a != b),
    }
}

fn range_header(start: u64, end: Option<u64>) -> HeaderMap {
    let mut h = HeaderMap::new();
    let v = match end {
        Some(end) => format!("bytes={start}-{}", end - 1),
        None => format!("bytes={start}-"),
    };
    h.insert(header::RANGE, v.parse().expect("ascii range"));
    h
}

struct Source {
    /// Where the first request ended up after redirects.
    url: String,
    first: Option<Response>,
    is_playlist: bool,
}

/// Learn size, range support and name with one request, and keep that response
/// as the first connection (some links work only once).
async fn probe(client: &Client, ctx: &JobContext, task: &Shared) -> Result<Source> {
    let req = task.lock().unwrap().req.clone();
    let mut resp = send(client, &ctx.settings, &req, &req.url, &range_header(0, None)).await?;
    if resp.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        resp = send(client, &ctx.settings, &req, &req.url, &HeaderMap::new()).await?;
    }
    let status = resp.status();
    if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
        return Err(anyhow::Error::new(BadStatus(status)));
    }
    let h = resp.headers();
    let (total, resumable) = if status == StatusCode::PARTIAL_CONTENT {
        match header_str(h, header::CONTENT_RANGE).as_deref().and_then(parse_content_range) {
            Some((0, total)) => (Some(total), true),
            _ => bail!("server sent an unusable Content-Range"),
        }
    } else {
        (resp.content_length(), false)
    };
    let url = resp.url().to_string();
    let is_playlist = header_str(h, header::CONTENT_TYPE)
        .is_some_and(|t| t.to_ascii_lowercase().contains("mpegurl"));
    let disposition = h
        .get(header::CONTENT_DISPOSITION)
        .map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    {
        let mut t = task.lock().unwrap();
        t.file_name = crate::naming::guess_name(req.filename.as_deref(), disposition.as_deref(), &url);
        t.category = crate::naming::category(&t.file_name).to_string();
        t.total = total;
        t.resumable = Some(resumable);
        t.etag = header_str(h, header::ETAG).filter(|e| !e.starts_with("W/"));
        t.last_modified = header_str(h, header::LAST_MODIFIED);
    }
    Ok(Source { url, first: Some(resp), is_playlist })
}

fn destination(settings: &Settings, task: &Task) -> std::path::PathBuf {
    if let Some(d) = &task.req.dir {
        d.clone()
    } else if settings.category_folders && task.category != "General" {
        settings.download_dir.join(&task.category)
    } else {
        settings.download_dir.clone()
    }
}

pub fn ensure_destination(settings: &Settings, task: &mut Task) -> Result<()> {
    if task.dir.is_none() {
        task.dir = Some(destination(settings, task));
    }
    let dir = task.dir.as_ref().expect("just set");
    std::fs::create_dir_all(dir).with_context(|| format!("create folder {}", dir.display()))
}

/// Can the saved partial file be continued?
fn can_resume(task: &Task) -> bool {
    let (Some(total), Some(true), Some(path)) = (task.total, task.resumable, task.part_path()) else {
        return false;
    };
    task.segments.covers(total)
        && task.segments.progress_is_sane()
        && std::fs::metadata(path).is_ok_and(|m| m.len() == total)
}

pub async fn download(ctx: JobContext, task: Shared) -> Result<()> {
    let client = build_client(&ctx.settings)?;
    let resume = {
        let mut t = task.lock().unwrap();
        t.segments.reset_runtime();
        let ok = can_resume(&t);
        if !ok {
            if t.segments.downloaded() > 0 {
                t.note = Some("Partial data could not be resumed; started again.".into());
            }
            if let Some(p) = t.part_path() {
                let _ = std::fs::remove_file(p);
            }
            t.reset_progress();
        }
        ok
    };

    let mut source = if resume {
        let url = task.lock().unwrap().req.url.clone();
        Source { url, first: None, is_playlist: false }
    } else {
        let mut attempt = 0;
        loop {
            match probe(&client, &ctx, &task).await {
                Ok(s) => break s,
                Err(e) if !is_permanent(&e) && attempt < ctx.settings.max_retries => {
                    attempt += 1;
                    tokio::time::sleep(backoff(attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
    };

    if source.is_playlist {
        drop(source);
        task.lock().unwrap().reset_progress();
        return crate::hls::download(ctx, task).await;
    }

    let file = {
        let mut t = task.lock().unwrap();
        ensure_destination(&ctx.settings, &mut t)?;
        let path = t.part_path().expect("destination set");
        let file = OpenOptions::new().read(true).write(true).create(true).truncate(!resume).open(&path)?;
        if !resume {
            if let Some(total) = t.total {
                file.set_len(total).context("reserve disk space")?;
            }
            t.segments = SegmentMap::new(t.total);
        }
        Arc::new(file)
    };

    let (ranged, connections) = {
        let t = task.lock().unwrap();
        let ranged = t.resumable == Some(true) && t.total.is_some();
        let n = t.req.connections.unwrap_or(ctx.settings.max_connections).clamp(1, MAX_CONNECTIONS);
        (ranged, if ranged { n } else { 1 })
    };

    if task.lock().unwrap().total != Some(0) {
        if ranged {
            run_workers(&ctx, &task, &client, &mut source, &file, connections).await?;
        } else {
            run_single(&ctx, &task, &client, &mut source, &file).await?;
        }
    }

    file.sync_all().context("flush finished file")?;
    drop(file);
    finish(&task)
}

fn finish(task: &Shared) -> Result<()> {
    let mut t = task.lock().unwrap();
    if let Some(total) = t.total {
        if total > 0 && !(t.segments.covers(total) && t.segments.is_complete()) {
            bail!("internal error: download ended with missing data");
        }
    } else {
        t.total = Some(t.segments.downloaded());
    }
    let (part, dir) = (t.part_path().context("no part file")?, t.dir.clone().context("no folder")?);
    let target = crate::fsio::move_unique(&part, &dir, &t.file_name)?;
    if let Some(name) = target.file_name() {
        t.file_name = name.to_string_lossy().into_owned();
    }
    Ok(())
}

/// Keeps saved progress behind what is durably on disk: snapshot first, flush,
/// then record the snapshot.
async fn checkpoint_loop(task: Shared, file: Arc<File>, save: SaveFn) {
    loop {
        tokio::time::sleep(CHECKPOINT_EVERY).await;
        let snapshot = task.lock().unwrap().clone();
        let f = file.clone();
        if let Ok(Ok(())) = tokio::task::spawn_blocking(move || f.sync_data()).await {
            save(&snapshot);
        }
    }
}

/// Copy a response body into one segment until the segment or the body ends.
async fn pump(
    ctx: &JobContext,
    task: &Shared,
    file: &File,
    resp: Response,
    idx: usize,
) -> Result<()> {
    let mut body = resp.bytes_stream();
    loop {
        let next = tokio::time::timeout(IDLE_TIMEOUT, body.next())
            .await
            .map_err(|_| anyhow!("connection stalled"))?;
        let Some(chunk) = next else { return Ok(()) };
        let chunk = chunk.map_err(|e| anyhow!(e.without_url()))?;
        let mut rest: &[u8] = &chunk;
        while !rest.is_empty() {
            // Small slices keep the speed limiter smooth.
            let want = rest.len().min(64 * 1024);
            let Some((offset, n)) = task.lock().unwrap().segments.reserve(idx, want) else {
                return Ok(());
            };
            ctx.global_limit.acquire(n).await;
            ctx.task_limit.acquire(n).await;
            crate::fsio::write_at(file, &rest[..n], offset)?;
            task.lock().unwrap().segments.commit(idx, n);
            rest = &rest[n..];
        }
        if task.lock().unwrap().segments.segs[idx].is_complete() {
            return Ok(());
        }
    }
}

async fn fetch_range(
    ctx: &JobContext,
    task: &Shared,
    client: &Client,
    url: &str,
    file: &File,
    idx: usize,
) -> Result<()> {
    let (req, start, end, total, etag, modified) = {
        let t = task.lock().unwrap();
        let s = &t.segments.segs[idx];
        (t.req.clone(), s.position(), s.end, t.total, t.etag.clone(), t.last_modified.clone())
    };
    // No If-Range: some CDNs answer it with the whole file even when nothing
    // changed. The validators on the 206 reply are compared instead, before any
    // byte is written.
    let headers = range_header(start, Some(end));
    let resp = send(client, &ctx.settings, &req, url, &headers).await?;
    match resp.status() {
        StatusCode::PARTIAL_CONTENT => {}
        StatusCode::OK => {
            return Err(fatal(
                "The file on the server changed or it stopped supporting resume. Use Restart.",
            ))
        }
        s => return Err(anyhow::Error::new(BadStatus(s))),
    }
    let range = header_str(resp.headers(), header::CONTENT_RANGE).as_deref().and_then(parse_content_range);
    let new_etag = header_str(resp.headers(), header::ETAG);
    let new_modified = header_str(resp.headers(), header::LAST_MODIFIED);
    if range.map(|r| r.1) != total
        || validator_changed(&etag, &new_etag, &modified, &new_modified)
    {
        return Err(fatal("The file on the server changed. Use Restart."));
    }
    if range.map(|r| r.0) != Some(start) {
        bail!("server returned the wrong byte range");
    }
    pump(ctx, task, file, resp, idx).await?;
    if !task.lock().unwrap().segments.segs[idx].is_complete() {
        bail!("connection closed early");
    }
    Ok(())
}

struct Worker {
    ctx: JobContext,
    task: Shared,
    client: Client,
    /// Redirect target from the probe; dropped if it stops working.
    url: Arc<Mutex<String>>,
    file: Arc<File>,
}

impl Worker {
    async fn run(self, mut first: Option<(usize, Response)>) -> Result<()> {
        let min_split = self.ctx.settings.min_split.max(16 * 1024);
        let mut failures = 0u32;
        loop {
            let (idx, response) = match first.take() {
                Some((i, r)) => (i, Some(r)),
                None => match self.task.lock().unwrap().segments.next_work(min_split) {
                    Some(i) => (i, None),
                    None => return Ok(()),
                },
            };
            let before = self.task.lock().unwrap().segments.segs[idx].done;
            let result = match response {
                Some(r) => match pump(&self.ctx, &self.task, &self.file, r, idx).await {
                    Ok(()) if !self.task.lock().unwrap().segments.segs[idx].is_complete() => {
                        Err(anyhow!("connection closed early"))
                    }
                    other => other,
                },
                None => {
                    let url = self.url.lock().unwrap().clone();
                    fetch_range(&self.ctx, &self.task, &self.client, &url, &self.file, idx).await
                }
            };
            let (progressed, others) = {
                let mut t = self.task.lock().unwrap();
                let progressed = t.segments.segs[idx].done > before;
                t.segments.release(idx);
                (progressed, t.segments.active_count())
            };
            let Err(e) = result else {
                failures = 0;
                continue;
            };
            if e.downcast_ref::<Fatal>().is_some() || e.downcast_ref::<std::io::Error>().is_some() {
                return Err(e);
            }
            failures = if progressed { 1 } else { failures + 1 };
            if let Some(BadStatus(status)) = e.downcast_ref::<BadStatus>() {
                // A refused extra connection usually means a per-client limit:
                // retire this worker and let the others carry on.
                let limit = matches!(status.as_u16(), 403 | 429 | 503 | 509);
                if limit && others > 0 {
                    return Ok(());
                }
                // An expired redirect target: go back through the original URL.
                let original = self.task.lock().unwrap().req.url.clone();
                let mut url = self.url.lock().unwrap();
                if *url != original {
                    *url = original;
                } else if is_permanent(&e) {
                    return Err(e);
                }
            }
            if failures > self.ctx.settings.max_retries {
                return if others > 0 { Ok(()) } else { Err(e) };
            }
            tokio::time::sleep(backoff(failures)).await;
        }
    }
}

async fn run_workers(
    ctx: &JobContext,
    task: &Shared,
    client: &Client,
    source: &mut Source,
    file: &Arc<File>,
    connections: usize,
) -> Result<()> {
    let url = Arc::new(Mutex::new(source.url.clone()));
    let mut first = source.first.take().map(|r| {
        task.lock().unwrap().segments.take(0);
        (0, r)
    });
    // Dropping a JoinSet aborts its tasks, so pausing stops every connection.
    let mut workers = JoinSet::new();
    let mut checkpoints = JoinSet::new();
    checkpoints.spawn(checkpoint_loop(task.clone(), file.clone(), ctx.save.clone()));
    // Several rounds: workers retired by a server limit may leave work behind.
    let mut stalled_rounds = 0;
    loop {
        let before = task.lock().unwrap().segments.downloaded();
        for _ in 0..connections {
            let w = Worker {
                ctx: ctx.clone(),
                task: task.clone(),
                client: client.clone(),
                url: url.clone(),
                file: file.clone(),
            };
            workers.spawn(w.run(first.take()));
        }
        while let Some(joined) = workers.join_next().await {
            joined.context("download worker crashed")??;
        }
        let t = task.lock().unwrap();
        if t.segments.is_complete() {
            return Ok(());
        }
        stalled_rounds = if t.segments.downloaded() > before { 0 } else { stalled_rounds + 1 };
        if stalled_rounds >= 2 {
            bail!("no connection could finish the download");
        }
    }
}

/// Servers without range support get one connection; a failure restarts at zero.
async fn run_single(
    ctx: &JobContext,
    task: &Shared,
    client: &Client,
    source: &mut Source,
    file: &Arc<File>,
) -> Result<()> {
    let mut attempt = 0;
    loop {
        let result: Result<()> = async {
            let resp = match source.first.take() {
                Some(r) => r,
                None => {
                    let req = task.lock().unwrap().req.clone();
                    let r = send(client, &ctx.settings, &req, &req.url, &HeaderMap::new()).await?;
                    if r.status() != StatusCode::OK {
                        return Err(anyhow::Error::new(BadStatus(r.status())));
                    }
                    r
                }
            };
            let total = task.lock().unwrap().total;
            if total.is_some() && resp.content_length().is_some() && resp.content_length() != total {
                return Err(fatal("The file on the server changed. Use Restart."));
            }
            task.lock().unwrap().segments.take(0);
            pump(ctx, task, file, resp, 0).await?;
            let mut t = task.lock().unwrap();
            if t.total.is_none() {
                t.segments.seal(0);
                file.set_len(t.segments.downloaded())?;
            } else if !t.segments.is_complete() {
                bail!("connection closed early");
            }
            Ok(())
        }
        .await;
        match result {
            Ok(()) => return Ok(()),
            Err(e) if !is_permanent(&e) && attempt < ctx.settings.max_retries => {
                attempt += 1;
                tokio::time::sleep(backoff(attempt)).await;
                let mut t = task.lock().unwrap();
                t.segments = SegmentMap::new(t.total);
            }
            Err(e) => return Err(e),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn content_range() {
        assert_eq!(parse_content_range("bytes 5-9/10"), Some((5, 10)));
        assert_eq!(parse_content_range("bytes */10"), None);
    }

    #[test]
    fn origin_check_protects_credentials() {
        assert!(same_origin("https://a.com/x", "https://a.com:443/y"));
        assert!(!same_origin("https://a.com/x", "http://a.com/x"));
        assert!(!same_origin("https://a.com/x", "https://cdn.a.com/x"));
    }
}
