//! HTTP Live Streaming: pick the best variant, fetch media segments in
//! parallel, undo standard AES-128 encryption and join the result. Content
//! protected by DRM is refused.

use crate::http::{backoff, ensure_destination, fatal, send, BadStatus, Fatal, JobContext, Shared};
use crate::types::{DownloadRequest, MAX_CONNECTIONS};
use anyhow::{anyhow, bail, Context, Result};
use aes::cipher::{block_padding::Pkcs7, BlockModeDecrypt, KeyIvInit};
use futures_util::StreamExt;
use m3u8_rs::{AlternativeMediaType, ByteRange, KeyMethod, MasterPlaylist, MediaPlaylist, Playlist};
use reqwest::header::{self, HeaderMap};
use reqwest::{Client, StatusCode};
use sha2::{Digest, Sha256};
use std::collections::hash_map::Entry;
use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::task::JoinSet;

const MAX_BODY: usize = 512 * 1024 * 1024;

pub fn looks_like_playlist(url: &str) -> bool {
    url::Url::parse(url).is_ok_and(|u| u.path().to_ascii_lowercase().ends_with(".m3u8"))
}

#[derive(Clone)]
struct Piece {
    url: String,
    range: Option<(u64, u64)>,
    /// Key URL and initialisation vector when AES-128 encrypted.
    crypt: Option<(String, [u8; 16])>,
}

struct Track {
    pieces: Vec<Piece>,
    /// Initialisation section to insert before the piece with this index.
    inits: Vec<(usize, Piece)>,
}

#[derive(Clone)]
struct Fetcher {
    ctx: JobContext,
    client: Client,
    req: DownloadRequest,
}

impl Fetcher {
    async fn once(&self, url: &str, range: Option<(u64, u64)>) -> Result<(Vec<u8>, String)> {
        let mut headers = HeaderMap::new();
        if let Some((start, len)) = range {
            headers.insert(header::RANGE, format!("bytes={start}-{}", start + len - 1).parse()?);
        }
        let resp = send(&self.client, &self.ctx.settings, &self.req, url, &headers).await?;
        let wanted = if range.is_some() { StatusCode::PARTIAL_CONTENT } else { StatusCode::OK };
        if resp.status() != wanted {
            return Err(anyhow::Error::new(BadStatus(resp.status())));
        }
        let final_url = resp.url().to_string();
        let expected = resp.content_length();
        let mut body = resp.bytes_stream();
        let mut data = Vec::new();
        loop {
            let next = tokio::time::timeout(Duration::from_secs(45), body.next())
                .await
                .map_err(|_| anyhow!("connection stalled"))?;
            let Some(chunk) = next else { break };
            let chunk = chunk.map_err(|e| anyhow!(e.without_url()))?;
            if data.len() + chunk.len() > MAX_BODY {
                return Err(fatal("a stream segment is unreasonably large"));
            }
            self.ctx.global_limit.acquire(chunk.len()).await;
            self.ctx.task_limit.acquire(chunk.len()).await;
            data.extend_from_slice(&chunk);
        }
        if expected.is_some_and(|n| n != data.len() as u64)
            || range.is_some_and(|r| r.1 != data.len() as u64)
        {
            bail!("segment was truncated");
        }
        Ok((data, final_url))
    }

    async fn get(&self, url: &str, range: Option<(u64, u64)>) -> Result<(Vec<u8>, String)> {
        let mut attempt = 0;
        loop {
            match self.once(url, range).await {
                Ok(v) => return Ok(v),
                Err(e) if e.downcast_ref::<Fatal>().is_none() && attempt < self.ctx.settings.max_retries => {
                    attempt += 1;
                    tokio::time::sleep(backoff(attempt)).await;
                }
                Err(e) => return Err(e),
            }
        }
    }

    async fn playlist(&self, url: &str) -> Result<(Playlist, String)> {
        let (data, final_url) = self.get(url, None).await?;
        let list = m3u8_rs::parse_playlist_res(&data)
            .map_err(|_| fatal("the address is not a readable HLS playlist"))?;
        Ok((list, final_url))
    }

    async fn media(&self, url: &str) -> Result<(MediaPlaylist, String)> {
        match self.playlist(url).await? {
            (Playlist::MediaPlaylist(m), u) => Ok((m, u)),
            _ => Err(fatal("expected a media playlist")),
        }
    }
}

fn resolve(base: &str, reference: &str) -> Result<String> {
    let joined = url::Url::parse(base)?.join(reference).context("bad URL in playlist")?;
    if !matches!(joined.scheme(), "http" | "https") {
        return Err(fatal("playlist refers to a non-web address"));
    }
    Ok(joined.to_string())
}

fn parse_iv(text: &str) -> Option<[u8; 16]> {
    let hex = text.strip_prefix("0x").or_else(|| text.strip_prefix("0X"))?;
    u128::from_str_radix(hex, 16).ok().map(u128::to_be_bytes)
}

/// Turns relative byte ranges into absolute ones.
fn absolute_range(r: &ByteRange, next: &mut u64) -> (u64, u64) {
    let start = r.offset.unwrap_or(*next);
    *next = start + r.length;
    (start, r.length)
}

fn plan(list: &MediaPlaylist, base: &str) -> Result<Track> {
    if !list.end_list {
        return Err(fatal("This is a live stream. Only finished (VOD) streams can be downloaded."));
    }
    if list.segments.is_empty() {
        return Err(fatal("the playlist has no segments"));
    }
    let mut pieces = Vec::new();
    let mut inits = Vec::new();
    let mut key: Option<(String, Option<[u8; 16]>)> = None;
    let mut next_offset = 0;
    let mut current_map = None;
    for (i, seg) in list.segments.iter().enumerate() {
        if let Some(k) = &seg.key {
            let standard = k.keyformat.as_deref().is_none_or(|f| f == "identity");
            key = match &k.method {
                KeyMethod::None => None,
                KeyMethod::AES128 if standard => {
                    let uri = k.uri.as_deref().ok_or_else(|| fatal("encrypted stream without a key"))?;
                    let iv = match &k.iv {
                        Some(v) => Some(parse_iv(v).ok_or_else(|| fatal("bad IV in playlist"))?),
                        None => None,
                    };
                    Some((resolve(base, uri)?, iv))
                }
                _ => return Err(fatal("This stream is DRM protected and is not supported.")),
            };
        }
        let sequence_iv = u128::from(list.media_sequence + i as u64).to_be_bytes();
        let crypt = key.as_ref().map(|(u, iv)| (u.clone(), iv.unwrap_or(sequence_iv)));
        if let Some(map) = &seg.map {
            if current_map.as_ref() != Some(map) {
                current_map = Some(map.clone());
                let range = map.byte_range.as_ref().map(|r| (r.offset.unwrap_or(0), r.length));
                // An encrypted init section requires an explicit IV.
                let init_crypt = key.as_ref().and_then(|(u, iv)| iv.map(|iv| (u.clone(), iv)));
                inits.push((i, Piece { url: resolve(base, &map.uri)?, range, crypt: init_crypt }));
            }
        }
        let range = seg.byte_range.as_ref().map(|r| absolute_range(r, &mut next_offset));
        pieces.push(Piece { url: resolve(base, &seg.uri)?, range, crypt });
    }
    Ok(Track { pieces, inits })
}

/// Highest bandwidth video variant, plus its separate audio playlist if any.
fn choose(master: &MasterPlaylist) -> Option<(String, Option<String>)> {
    let best = master.variants.iter().filter(|v| !v.is_i_frame).max_by_key(|v| v.bandwidth)?;
    let audio = best.audio.as_ref().and_then(|group| {
        let options: Vec<_> = master
            .alternatives
            .iter()
            .filter(|a| a.media_type == AlternativeMediaType::Audio && &a.group_id == group)
            .filter(|a| a.uri.is_some())
            .collect();
        options.iter().find(|a| a.default).or(options.first()).and_then(|a| a.uri.clone())
    });
    Some((best.uri.clone(), audio))
}

fn decrypt(data: Vec<u8>, key: &[u8; 16], iv: &[u8; 16]) -> Result<Vec<u8>> {
    cbc::Decryptor::<aes::Aes128>::new(&(*key).into(), &(*iv).into())
        .decrypt_padded_vec::<Pkcs7>(&data)
        .map_err(|_| fatal("could not decrypt a segment (wrong key?)"))
}

async fn fetch_piece(f: &Fetcher, piece: &Piece, keys: &HashMap<String, [u8; 16]>) -> Result<Vec<u8>> {
    let (data, _) = f.get(&piece.url, piece.range).await?;
    match &piece.crypt {
        Some((key_url, iv)) => decrypt(data, &keys[key_url], iv),
        None => Ok(data),
    }
}

/// Changes whenever the playlist no longer describes the saved pieces. Query
/// strings are ignored because they often carry short-lived tokens.
fn fingerprint(tracks: &[Track]) -> String {
    let mut h = Sha256::new();
    for t in tracks {
        h.update(b"track");
        for p in &t.pieces {
            let path = p.url.split(['?', '#']).next().unwrap_or("");
            h.update(format!("{path} {:?}\n", p.range));
        }
    }
    h.finalize().iter().map(|b| format!("{b:02x}")).collect()
}

fn piece_path(dir: &Path, track: usize, index: usize) -> PathBuf {
    dir.join(format!("{track}-{index:06}.seg"))
}

fn join_track(dir: &Path, track: usize, count: usize, inits: &[(usize, Vec<u8>)], out: &Path) -> Result<()> {
    let mut file = std::fs::File::create(out)?;
    for i in 0..count {
        for (_, data) in inits.iter().filter(|(at, _)| *at == i) {
            file.write_all(data)?;
        }
        std::io::copy(&mut std::fs::File::open(piece_path(dir, track, i))?, &mut file)?;
    }
    file.sync_all()?;
    Ok(())
}

/// Apps started from Finder or by a browser get a minimal `PATH`, so also look
/// where package managers install it.
fn ffmpeg_path() -> PathBuf {
    ["/opt/homebrew/bin/ffmpeg", "/usr/local/bin/ffmpeg"]
        .into_iter()
        .map(PathBuf::from)
        .find(|p| p.is_file())
        .unwrap_or_else(|| "ffmpeg".into())
}

/// Merge separate video and audio without re-encoding, if ffmpeg is installed.
async fn mux(video: &Path, audio: &Path, out: &Path) -> bool {
    let status = tokio::process::Command::new(ffmpeg_path())
        .args(["-nostdin", "-loglevel", "error", "-y", "-i"])
        .arg(video)
        .arg("-i")
        .arg(audio)
        .args(["-map", "0:v?", "-map", "1:a", "-c", "copy", "-f", "mp4"])
        .arg(out)
        .kill_on_drop(true)
        .status()
        .await;
    status.is_ok_and(|s| s.success())
}

pub fn parts_dir(task: &crate::types::Task) -> Option<PathBuf> {
    task.part_path().map(|p| p.with_extension("hlsparts"))
}

pub async fn download(ctx: JobContext, task: Shared) -> Result<()> {
    let client = crate::http::build_client(&ctx.settings)?;
    let req = task.lock().unwrap().req.clone();
    let fetcher = Fetcher { ctx: ctx.clone(), client, req: req.clone() };

    let (first, base) = fetcher.playlist(&req.url).await?;
    let mut tracks = Vec::new();
    match first {
        Playlist::MediaPlaylist(m) => tracks.push(plan(&m, &base)?),
        Playlist::MasterPlaylist(master) => {
            let (video, audio) = choose(&master).ok_or_else(|| fatal("no playable variant"))?;
            for reference in std::iter::once(video).chain(audio) {
                let (m, u) = fetcher.media(&resolve(&base, &reference)?).await?;
                tracks.push(plan(&m, &u)?);
            }
        }
    }

    let fragmented = !tracks[0].inits.is_empty();
    let dir = {
        let mut t = task.lock().unwrap();
        // Extension follows the container the segments already use.
        if t.parts_total == 0 {
            let name = crate::naming::guess_name(req.filename.as_deref(), None, &req.url);
            let stem = name.rsplit_once('.').map_or(name.as_str(), |(s, _)| s);
            t.file_name = format!("{stem}.{}", if fragmented { "mp4" } else { "ts" });
            t.category = "Video".into();
        }
        ensure_destination(&ctx.settings, &mut t)?;
        parts_dir(&t).expect("destination set")
    };

    let plan_file = dir.join("plan");
    let print = fingerprint(&tracks);
    if std::fs::read_to_string(&plan_file).ok().as_deref() != Some(&print) {
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir)?;
        std::fs::write(&plan_file, &print)?;
    }

    // Keys are fetched up front and kept only in memory.
    let mut keys = HashMap::new();
    let mut key_urls = Vec::new();
    for track in &tracks {
        let inits = track.inits.iter().map(|(_, p)| p);
        for piece in track.pieces.iter().chain(inits) {
            if let Some((url, _)) = &piece.crypt {
                key_urls.push(url.clone());
            }
        }
    }
    for url in key_urls {
        if let Entry::Vacant(slot) = keys.entry(url) {
            let (data, _) = fetcher.get(slot.key(), None).await?;
            slot.insert(data.try_into().map_err(|_| fatal("invalid decryption key"))?);
        }
    }
    let keys = Arc::new(keys);

    let mut queue = Vec::new();
    let (mut done, mut bytes) = (0, 0);
    for (t, track) in tracks.iter().enumerate() {
        for (i, piece) in track.pieces.iter().enumerate() {
            match std::fs::metadata(piece_path(&dir, t, i)) {
                Ok(m) => {
                    done += 1;
                    bytes += m.len();
                }
                Err(_) => queue.push((t, i, piece.clone())),
            }
        }
    }
    {
        let mut t = task.lock().unwrap();
        t.parts_total = tracks.iter().map(|t| t.pieces.len()).sum();
        t.parts_done = done;
        t.stream_bytes = bytes;
        t.resumable = Some(true);
        (ctx.save)(&t);
    }

    let queue = Arc::new(queue);
    let cursor = Arc::new(AtomicUsize::new(0));
    let connections = req.connections.unwrap_or(ctx.settings.max_connections).clamp(1, MAX_CONNECTIONS);
    let mut workers = JoinSet::new();
    for _ in 0..connections {
        let (queue, cursor, keys, fetcher, task, dir) =
            (queue.clone(), cursor.clone(), keys.clone(), fetcher.clone(), task.clone(), dir.clone());
        workers.spawn(async move {
            while let Some((t, i, piece)) = queue.get(cursor.fetch_add(1, Ordering::SeqCst)) {
                let data = fetch_piece(&fetcher, piece, &keys).await?;
                let path = piece_path(&dir, *t, *i);
                let tmp = path.with_extension("tmp");
                let mut file = std::fs::File::create(&tmp)?;
                file.write_all(&data)?;
                file.sync_data()?;
                std::fs::rename(&tmp, &path)?;
                let snapshot = {
                    let mut s = task.lock().unwrap();
                    s.parts_done += 1;
                    s.stream_bytes += data.len() as u64;
                    (s.parts_done % 10 == 0).then(|| s.clone())
                };
                if let Some(s) = snapshot {
                    (fetcher.ctx.save)(&s);
                }
            }
            anyhow::Ok(())
        });
    }
    while let Some(joined) = workers.join_next().await {
        joined.context("stream worker crashed")??;
    }

    let part = task.lock().unwrap().part_path().expect("destination set");
    let mut outputs = Vec::new();
    for (t, track) in tracks.iter().enumerate() {
        let mut inits = Vec::new();
        for (at, piece) in &track.inits {
            inits.push((*at, fetch_piece(&fetcher, piece, &keys).await?));
        }
        let out = dir.join(format!("track{t}.joined"));
        join_track(&dir, t, track.pieces.len(), &inits, &out)?;
        outputs.push(out);
    }

    let mut separate_audio = None;
    let mut muxed = false;
    if outputs.len() == 2 {
        if mux(&outputs[0], &outputs[1], &part).await {
            muxed = true;
        } else {
            separate_audio = Some(outputs[1].clone());
        }
    }
    if !muxed {
        std::fs::rename(&outputs[0], &part)?;
    }

    let mut t = task.lock().unwrap();
    let folder = t.dir.clone().expect("destination set");
    if muxed {
        let stem = t.file_name.rsplit_once('.').map_or(t.file_name.as_str(), |(s, _)| s);
        t.file_name = format!("{stem}.mp4");
    }
    let target = crate::fsio::move_unique(&part, &folder, &t.file_name)?;
    t.file_name = target.file_name().context("file name")?.to_string_lossy().into_owned();
    if let Some(audio) = separate_audio {
        let stem = t.file_name.rsplit_once('.').map_or(t.file_name.clone(), |(s, _)| s.to_string());
        let saved = crate::fsio::move_unique(&audio, &folder, &format!("{stem}.audio.m4a"))?;
        t.note = Some(format!(
            "Audio saved separately as {} (install ffmpeg to merge automatically).",
            saved.file_name().unwrap_or_default().to_string_lossy()
        ));
    }
    t.total = std::fs::metadata(&target).ok().map(|m| m.len());
    t.stream_bytes = t.total.unwrap_or(t.stream_bytes);
    let _ = std::fs::remove_dir_all(&dir);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn media(text: &str) -> MediaPlaylist {
        m3u8_rs::parse_media_playlist_res(text.as_bytes()).unwrap()
    }

    #[test]
    fn refuses_drm_and_live() {
        let drm = media("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-KEY:METHOD=SAMPLE-AES,URI=\"skd://x\",KEYFORMAT=\"com.apple.streamingkeydelivery\"\n#EXTINF:4,\na.ts\n#EXT-X-ENDLIST\n");
        assert!(plan(&drm, "http://h/p.m3u8").is_err());
        let live = media("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXTINF:4,\na.ts\n");
        assert!(plan(&live, "http://h/p.m3u8").is_err());
    }

    #[test]
    fn default_iv_is_the_media_sequence_number() {
        let m = media("#EXTM3U\n#EXT-X-TARGETDURATION:4\n#EXT-X-MEDIA-SEQUENCE:7\n#EXT-X-KEY:METHOD=AES-128,URI=\"k\"\n#EXTINF:4,\na.ts\n#EXTINF:4,\nb.ts\n#EXT-X-ENDLIST\n");
        let t = plan(&m, "http://h/d/p.m3u8").unwrap();
        let (url, iv) = t.pieces[1].crypt.clone().unwrap();
        assert_eq!(url, "http://h/d/k");
        assert_eq!(u128::from_be_bytes(iv), 8);
    }

    #[test]
    fn refuses_local_file_references() {
        assert!(resolve("http://h/p.m3u8", "file:///etc/passwd").is_err());
    }
}
