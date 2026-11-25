//! File names and categories. Names come from servers and web pages, so they are
//! always treated as untrusted.

use percent_encoding::percent_decode_str;

const MAX_NAME_BYTES: usize = 180;

pub fn sanitize(raw: &str) -> Option<String> {
    let base = raw.rsplit(['/', '\\']).next().unwrap_or("");
    let cleaned: String = base
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '|' | '?' | '*' => '_',
            c if c.is_control() => '_',
            // Bidirectional overrides can disguise an extension.
            '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}' | '\u{200e}' | '\u{200f}' => '_',
            c => c,
        })
        .collect();
    let trimmed = cleaned.trim_matches(|c: char| c == '.' || c.is_whitespace());
    if trimmed.is_empty() {
        return None;
    }
    let mut name = trimmed.to_string();
    let stem = name.split('.').next().unwrap_or("").to_ascii_uppercase();
    let reserved = matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || (stem.len() == 4
            && (stem.starts_with("COM") || stem.starts_with("LPT"))
            && stem.ends_with(|c: char| c.is_ascii_digit()));
    if reserved {
        name.insert(0, '_');
    }
    Some(truncate_keeping_extension(&name, MAX_NAME_BYTES))
}

fn split_extension(name: &str) -> (&str, &str) {
    match name.rfind('.') {
        Some(i) if i > 0 && name.len() - i <= 16 => name.split_at(i),
        _ => (name, ""),
    }
}

fn truncate_keeping_extension(name: &str, max: usize) -> String {
    if name.len() <= max {
        return name.to_string();
    }
    let (stem, ext) = split_extension(name);
    let mut cut = max - ext.len();
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{ext}", &stem[..cut])
}

/// `name`, `name (1)`, `name (2)`, ...
pub fn numbered(name: &str, n: u32) -> String {
    if n == 0 {
        return name.to_string();
    }
    let (stem, ext) = split_extension(name);
    format!("{stem} ({n}){ext}")
}

/// Parse a `Content-Disposition` value, preferring the RFC 5987 `filename*`.
pub fn from_content_disposition(value: &str) -> Option<String> {
    let mut plain = None;
    let mut parts = Vec::new();
    let (mut start, mut quoted, mut escaped) = (0, false, false);
    for (i, c) in value.char_indices() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                parts.push(&value[start..i]);
                start = i + 1;
            }
            _ => {}
        }
    }
    parts.push(&value[start..]);
    for part in parts {
        let Some((key, val)) = part.split_once('=') else { continue };
        let val = val.trim();
        match key.trim().to_ascii_lowercase().as_str() {
            "filename*" => {
                let mut it = val.splitn(3, '\'');
                if let (Some(charset), Some(_), Some(data)) = (it.next(), it.next(), it.next()) {
                    let bytes: Vec<u8> = percent_decode_str(data).collect();
                    let text = if charset.eq_ignore_ascii_case("iso-8859-1") {
                        bytes.iter().map(|&b| b as char).collect()
                    } else {
                        String::from_utf8_lossy(&bytes).into_owned()
                    };
                    return Some(text);
                }
            }
            "filename" => {
                let v = val.strip_prefix('"').and_then(|v| v.strip_suffix('"')).unwrap_or(val);
                plain = Some(v.replace("\\\"", "\""));
            }
            _ => {}
        }
    }
    plain
}

fn from_url(url: &str) -> Option<String> {
    let parsed = url::Url::parse(url).ok()?;
    let last = parsed.path_segments()?.rfind(|s| !s.is_empty())?.to_string();
    Some(percent_decode_str(&last).decode_utf8_lossy().into_owned())
}

/// Preference order: explicit name, server header, URL path.
pub fn guess_name(explicit: Option<&str>, disposition: Option<&str>, url: &str) -> String {
    explicit
        .and_then(sanitize)
        .or_else(|| disposition.and_then(from_content_disposition).and_then(|n| sanitize(&n)))
        .or_else(|| from_url(url).and_then(|n| sanitize(&n)))
        .unwrap_or_else(|| "download".to_string())
}

const MAX_BATCH: u64 = 1000;

/// Expand one `[first-last]` numeric pattern, keeping leading zeros:
/// `img[08-10].jpg` gives `img08.jpg`, `img09.jpg`, `img10.jpg`.
/// Text without such a pattern is returned unchanged.
pub fn expand_batch(url: &str) -> Result<Vec<String>, String> {
    for (open, _) in url.match_indices('[') {
        let Some(len) = url[open..].find(']') else { break };
        let inner = &url[open + 1..open + len];
        let Some((a, b)) = inner.split_once('-') else { continue };
        let digits = |s: &str| !s.is_empty() && s.bytes().all(|c| c.is_ascii_digit());
        if !digits(a) || !digits(b) {
            continue;
        }
        let (Ok(first), Ok(last)) = (a.parse::<u64>(), b.parse::<u64>()) else {
            return Err("batch numbers are too large".into());
        };
        if last < first || last - first >= MAX_BATCH {
            return Err(format!("a batch must count upwards and hold at most {MAX_BATCH} files"));
        }
        let width = if a.starts_with('0') { a.len() } else { 0 };
        return Ok((first..=last)
            .map(|n| format!("{}{n:0width$}{}", &url[..open], &url[open + len + 1..]))
            .collect());
    }
    Ok(vec![url.to_string()])
}

pub const CATEGORIES: [&str; 6] = ["Video", "Music", "Documents", "Compressed", "Programs", "General"];

pub fn category(name: &str) -> &'static str {
    let ext = name.rsplit_once('.').map(|(_, e)| e.to_ascii_lowercase()).unwrap_or_default();
    match ext.as_str() {
        "avi" | "mpg" | "mpe" | "mpeg" | "asf" | "wmv" | "mov" | "qt" | "rm" | "mp4" | "flv" | "m4v"
        | "webm" | "ogv" | "mkv" | "ts" | "tsv" | "m3u8" | "3gp" => "Video",
        "mp3" | "wav" | "wma" | "mpa" | "ram" | "ra" | "aac" | "aif" | "m4a" | "flac" | "ogg"
        | "opus" | "tsa" => "Music",
        "doc" | "docx" | "pdf" | "ppt" | "pptx" | "pps" | "xls" | "xlsx" | "txt" | "epub" | "csv" => {
            "Documents"
        }
        "zip" | "rar" | "7z" | "gz" | "tgz" | "bz2" | "xz" | "zst" | "tar" | "z" | "arj" | "lzh"
        | "sit" => "Compressed",
        "exe" | "msi" | "msix" | "apk" | "ipa" | "dmg" | "pkg" | "deb" | "rpm" | "appimage" | "bin"
        | "iso" | "img" => "Programs",
        _ => "General",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blocks_traversal_and_reserved_names() {
        assert_eq!(sanitize("../../etc/passwd").as_deref(), Some("passwd"));
        assert_eq!(sanitize("..\\..\\boot.ini").as_deref(), Some("boot.ini"));
        assert_eq!(sanitize(".."), None);
        assert_eq!(sanitize(".bashrc").as_deref(), Some("bashrc"));
        assert_eq!(sanitize("CON.txt").as_deref(), Some("_CON.txt"));
        assert_eq!(sanitize("a:b?.zip").as_deref(), Some("a_b_.zip"));
        let long = format!("{}.tar", "é".repeat(300));
        let s = sanitize(&long).unwrap();
        assert!(s.len() <= MAX_NAME_BYTES && s.ends_with(".tar"));
    }

    #[test]
    fn reads_content_disposition() {
        assert_eq!(
            from_content_disposition("attachment; filename=\"a;b.zip\"").as_deref(),
            Some("a;b.zip")
        );
        assert_eq!(
            from_content_disposition("attachment; filename=x.zip; filename*=UTF-8''%e2%82%ac.zip")
                .as_deref(),
            Some("€.zip")
        );
    }

    #[test]
    fn expands_batches() {
        assert_eq!(expand_batch("h/a[08-10].jpg").unwrap(), ["h/a08.jpg", "h/a09.jpg", "h/a10.jpg"]);
        assert_eq!(expand_batch("http://[::1]/x").unwrap(), ["http://[::1]/x"]);
        assert!(expand_batch("a[5-1]").is_err());
        assert!(expand_batch("a[0-100000]").is_err());
    }

    #[test]
    fn falls_back_to_url() {
        assert_eq!(guess_name(None, None, "http://h/p/My%20File.iso?x=1"), "My File.iso");
        assert_eq!(guess_name(None, None, "http://h/"), "download");
        assert_eq!(numbered("a.tar", 2), "a (2).tar");
    }
}
