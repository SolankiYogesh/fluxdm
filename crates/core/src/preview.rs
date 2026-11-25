//! What a download will be called, how big it is and where it will go, found
//! out before it is added.

use crate::types::{DownloadRequest, Settings};
use crate::{ftp, hls, http, naming};
use anyhow::{anyhow, bail, Context, Result};
use reqwest::header::{self, HeaderMap};
use reqwest::StatusCode;
use std::path::PathBuf;
use std::time::Duration;

const TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Debug, Clone, PartialEq)]
pub struct FileInfo {
    pub name: String,
    pub size: Option<u64>,
    /// `None` when the protocol does not tell us up front.
    pub resumable: Option<bool>,
    pub is_stream: bool,
    /// The address answered with a web page, not a file.
    pub is_web_page: bool,
}

/// The folder a file of this name would be saved in.
pub fn folder_for(settings: &Settings, name: &str) -> PathBuf {
    http::folder(settings, None, naming::category(name))
}

/// The name the finished file would really get, since existing files are never
/// overwritten.
pub fn free_name(dir: &std::path::Path, name: &str) -> String {
    (0..1000)
        .map(|n| naming::numbered(name, n))
        .find(|candidate| !dir.join(candidate).exists())
        .unwrap_or_else(|| name.to_string())
}

/// Is this a complete enough address to be worth asking a server about?
pub fn is_fetchable(address: &str) -> bool {
    url::Url::parse(address.trim()).is_ok_and(|u| {
        matches!(u.scheme(), "http" | "https" | "ftp") && u.host_str().is_some_and(|h| !h.is_empty())
    })
}

/// Ask the server about one address without downloading the file.
pub async fn inspect(settings: &Settings, req: &DownloadRequest) -> Result<FileInfo> {
    if !is_fetchable(&req.url) {
        bail!("enter a full http, https or ftp address");
    }
    tokio::time::timeout(TIMEOUT, ask(settings, req))
        .await
        .map_err(|_| anyhow!("the server took too long to answer"))?
}

async fn ask(settings: &Settings, req: &DownloadRequest) -> Result<FileInfo> {
    if req.url.get(..6).is_some_and(|s| s.eq_ignore_ascii_case("ftp://")) {
        return Ok(FileInfo {
            name: naming::guess_name(req.filename.as_deref(), None, &req.url),
            size: ftp::size_of(req).await?,
            resumable: None,
            is_stream: false,
            is_web_page: false,
        });
    }
    let client = http::build_client(settings).context("could not start a connection")?;
    let mut range = HeaderMap::new();
    range.insert(header::RANGE, "bytes=0-".parse().expect("ascii range"));
    let mut resp = http::send(&client, settings, req, &req.url, &range).await?;
    if resp.status() == StatusCode::RANGE_NOT_SATISFIABLE {
        resp = http::send(&client, settings, req, &req.url, &HeaderMap::new()).await?;
    }
    let status = resp.status();
    if status != StatusCode::OK && status != StatusCode::PARTIAL_CONTENT {
        bail!("server replied {status}");
    }
    let headers = resp.headers();
    let text = |name| headers.get(name).map(|v| String::from_utf8_lossy(v.as_bytes()).into_owned());
    let kind = text(header::CONTENT_TYPE).unwrap_or_default().to_ascii_lowercase();
    let disposition = text(header::CONTENT_DISPOSITION);
    let (size, resumable) = if status == StatusCode::PARTIAL_CONTENT {
        let total = text(header::CONTENT_RANGE).and_then(|v| v.rsplit_once('/')?.1.trim().parse().ok());
        (total, true)
    } else {
        (resp.content_length(), false)
    };
    let name = naming::guess_name(req.filename.as_deref(), disposition.as_deref(), resp.url().as_str());
    let is_stream = kind.contains("mpegurl") || hls::looks_like_playlist(&req.url);
    let attached = disposition.is_some_and(|d| d.trim_start().to_ascii_lowercase().starts_with("attachment"));
    Ok(FileInfo {
        name,
        // A playlist's length says nothing about the size of the video.
        size: size.filter(|_| !is_stream),
        resumable: (!is_stream).then_some(resumable),
        is_stream,
        is_web_page: kind.starts_with("text/html") && !attached,
    })
    // Dropping the response here closes the connection without reading the body.
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_full_addresses_are_fetched() {
        assert!(is_fetchable(" https://a.com/x.zip "));
        assert!(is_fetchable("ftp://host/f"));
        for partial in ["", "https://", "htt", "a.com/x.zip", "file:///etc/passwd", "javascript:1"] {
            assert!(!is_fetchable(partial), "{partial}");
        }
    }

    #[test]
    fn folder_follows_the_category_setting() {
        let mut s = Settings { download_dir: "/d".into(), category_folders: true, ..Default::default() };
        assert_eq!(folder_for(&s, "a.zip"), PathBuf::from("/d/Compressed"));
        assert_eq!(folder_for(&s, "a.unknown"), PathBuf::from("/d"));
        s.category_folders = false;
        assert_eq!(folder_for(&s, "a.zip"), PathBuf::from("/d"));
    }
}
