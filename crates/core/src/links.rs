//! Finding downloadable addresses in pasted text, web pages and the clipboard.

use crate::types::{DownloadRequest, Settings};
use anyhow::{anyhow, bail, Context, Result};
use futures_util::StreamExt;
use scraper::{Html, Selector};
use std::collections::HashSet;
use std::time::Duration;

const MAX_PAGE: usize = 8 * 1024 * 1024;
const MAX_LINKS: usize = 5000;

#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub url: String,
    pub name: String,
    pub category: &'static str,
    /// The address ends in a recognised file type, so it is probably a file.
    pub is_file: bool,
}

const PAGE_EXTENSIONS: [&str; 7] = ["html", "htm", "php", "asp", "aspx", "jsp", "cgi"];
const IMAGE_EXTENSIONS: [&str; 7] = ["jpg", "jpeg", "png", "gif", "webp", "svg", "bmp"];
const OTHER_FILES: [&str; 3] = ["torrent", "srt", "vtt"];

fn describe(url: url::Url) -> Option<Link> {
    if !matches!(url.scheme(), "http" | "https" | "ftp") || url.host_str().is_none() {
        return None;
    }
    let url = {
        let mut u = url;
        u.set_fragment(None);
        u.to_string()
    };
    let name = crate::naming::guess_name(None, None, &url);
    let category = crate::naming::category(&name);
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    let known = category != "General"
        || IMAGE_EXTENSIONS.contains(&ext.as_str())
        || OTHER_FILES.contains(&ext.as_str());
    let is_file = known && !PAGE_EXTENSIONS.contains(&ext.as_str());
    Some(Link { url, name, category, is_file })
}

fn unique(links: impl Iterator<Item = Link>) -> Vec<Link> {
    let mut seen = HashSet::new();
    links.filter(|l| seen.insert(l.url.clone())).take(MAX_LINKS).collect()
}

/// Every address mentioned anywhere in free text, such as a chat or a list.
/// Numeric batch patterns are expanded.
pub fn from_text(text: &str) -> Vec<Link> {
    let separators = |c: char| c.is_whitespace() || matches!(c, '"' | '\'' | '<' | '>' | '`');
    unique(
        text.split(separators)
            .filter_map(|word| {
                let at = ["https://", "http://", "ftp://"].iter().filter_map(|p| word.find(p)).min()?;
                Some(word[at..].trim_end_matches(['.', ',', ';', ')', '}']))
            })
            .flat_map(|word| crate::naming::expand_batch(word).unwrap_or_default())
            .filter_map(|u| url::Url::parse(&u).ok())
            .filter_map(describe),
    )
}

/// Links, images and media referenced by an HTML page.
pub fn from_html(base: &str, html: &str) -> Vec<Link> {
    let Ok(mut base) = url::Url::parse(base) else { return Vec::new() };
    let document = Html::parse_document(html);
    let select = |css: &str| Selector::parse(css).expect("fixed selector");
    if let Some(href) = document.select(&select("base[href]")).next().and_then(|e| e.value().attr("href")) {
        if let Ok(joined) = base.join(href) {
            base = joined;
        }
    }
    let found = select("a[href], area[href], img[src], video[src], audio[src], source[src], embed[src]");
    unique(
        document
            .select(&found)
            .filter_map(|e| e.value().attr("href").or_else(|| e.value().attr("src")))
            .filter_map(|r| base.join(r.trim()).ok())
            .filter_map(describe),
    )
}

/// Download one web page and list what it links to. Scripts are not run, so
/// links that a page adds with JavaScript are not found.
pub async fn grab_page(settings: &Settings, address: &str) -> Result<Vec<Link>> {
    let parsed = url::Url::parse(address.trim()).context("that is not a valid address")?;
    if !matches!(parsed.scheme(), "http" | "https") {
        bail!("only web pages can be searched for links");
    }
    let client = crate::http::build_client(settings)?;
    let req = DownloadRequest { url: parsed.to_string(), ..Default::default() };
    let response = crate::http::send(&client, settings, &req, &req.url, &Default::default()).await?;
    if !response.status().is_success() {
        bail!("server replied {}", response.status());
    }
    let base = response.url().to_string();
    let mut body = response.bytes_stream();
    let mut data = Vec::new();
    loop {
        let next = tokio::time::timeout(Duration::from_secs(45), body.next())
            .await
            .map_err(|_| anyhow!("the page stopped responding"))?;
        let Some(chunk) = next else { break };
        data.extend_from_slice(&chunk.map_err(|e| anyhow!(e.without_url()))?);
        if data.len() > MAX_PAGE {
            bail!("the page is too large to search");
        }
    }
    Ok(from_html(&base, &String::from_utf8_lossy(&data)))
}

/// The clipboard is acted on only when it is exactly one address of a file.
/// Images are left alone because copying picture addresses is common.
pub fn clipboard_candidate(text: &str) -> Option<String> {
    let text = text.trim();
    if text.len() > 4096 || text.contains(char::is_whitespace) {
        return None;
    }
    let link = describe(url::Url::parse(text).ok()?)?;
    (link.is_file && link.category != "General").then(|| text.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_links_in_free_text() {
        let found = from_text("get (https://a.com/x.zip), and <http://b.com/p[1-3].mp3> https://a.com/x.zip");
        let urls: Vec<_> = found.iter().map(|l| l.url.as_str()).collect();
        assert_eq!(
            urls,
            ["https://a.com/x.zip", "http://b.com/p1.mp3", "http://b.com/p2.mp3", "http://b.com/p3.mp3"]
        );
    }

    #[test]
    fn reads_a_page_and_resolves_relative_links() {
        let html = r##"<base href="/files/"><a href="a.pdf#p2">A</a><a href="page.html">B</a>
            <a href="javascript:x()">C</a><a href="mailto:a@b.c">D</a><img src="//cdn.test/i.png">"##;
        let found = from_html("https://site.test/dir/index.html", html);
        let rows: Vec<_> = found.iter().map(|l| (l.url.as_str(), l.is_file)).collect();
        assert_eq!(
            rows,
            [
                ("https://site.test/files/a.pdf", true),
                ("https://site.test/files/page.html", false),
                ("https://cdn.test/i.png", true),
            ]
        );
    }

    #[test]
    fn clipboard_only_offers_single_file_addresses() {
        assert!(clipboard_candidate(" https://a.com/setup.dmg \n").is_some());
        assert!(clipboard_candidate("https://a.com/article").is_none());
        assert!(clipboard_candidate("https://a.com/photo.jpg").is_none());
        assert!(clipboard_candidate("see https://a.com/a.zip").is_none());
        assert!(clipboard_candidate("my password").is_none());
    }
}
