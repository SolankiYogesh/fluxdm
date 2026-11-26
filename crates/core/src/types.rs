use crate::segments::SegmentMap;
use serde::{Deserialize, Serialize};
use std::path::PathBuf;

/// What the user (or the browser) asked for.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct DownloadRequest {
    pub url: String,
    pub referrer: Option<String>,
    pub cookie: Option<String>,
    pub user_agent: Option<String>,
    pub username: Option<String>,
    pub password: Option<String>,
    /// Preferred file name (sanitised before use).
    pub filename: Option<String>,
    /// Destination folder; defaults to the category folder.
    pub dir: Option<PathBuf>,
    /// `sha256:<hex>`, `sha1:<hex>` or `md5:<hex>`.
    pub checksum: Option<String>,
    /// Per-download connection count override.
    pub connections: Option<usize>,
    /// Per-download speed limit in bytes per second, 0 = unlimited.
    pub speed_limit: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Status {
    Queued,
    Downloading,
    Paused,
    Completed,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: u64,
    pub req: DownloadRequest,
    pub status: Status,
    pub file_name: String,
    pub dir: Option<PathBuf>,
    pub category: String,
    pub total: Option<u64>,
    pub resumable: Option<bool>,
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    pub segments: SegmentMap,
    /// Stream downloads count whole media segments instead of byte ranges.
    pub parts_done: usize,
    pub parts_total: usize,
    pub stream_bytes: u64,
    pub error: Option<String>,
    pub note: Option<String>,
    pub added_at: i64,
    pub completed_at: Option<i64>,
    /// Started by hand: ignores the scheduler window.
    pub force: bool,
}

impl Task {
    pub fn new(id: u64, req: DownloadRequest) -> Self {
        let file_name = crate::naming::guess_name(req.filename.as_deref(), None, &req.url);
        let category = crate::naming::category(&file_name).to_string();
        Self {
            id,
            req,
            status: Status::Queued,
            file_name,
            dir: None,
            category,
            total: None,
            resumable: None,
            etag: None,
            last_modified: None,
            segments: SegmentMap::default(),
            parts_done: 0,
            parts_total: 0,
            stream_bytes: 0,
            error: None,
            note: None,
            added_at: chrono::Utc::now().timestamp(),
            completed_at: None,
            force: false,
        }
    }

    pub fn downloaded(&self) -> u64 {
        if self.parts_total > 0 {
            self.stream_bytes
        } else {
            self.segments.downloaded()
        }
    }

    pub fn fraction(&self) -> Option<f32> {
        if self.status == Status::Completed {
            return Some(1.0);
        }
        if self.parts_total > 0 {
            return Some(self.parts_done as f32 / self.parts_total as f32);
        }
        match self.total {
            Some(t) if t > 0 => Some((self.segments.downloaded() as f64 / t as f64) as f32),
            _ => None,
        }
    }

    /// In-progress data file, kept next to the destination.
    pub fn part_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(format!("{}.{}.fluxpart", self.file_name, self.id)))
    }

    pub fn final_path(&self) -> Option<PathBuf> {
        self.dir.as_ref().map(|d| d.join(&self.file_name))
    }

    /// Discard all progress so the next run starts from byte zero.
    pub fn reset_progress(&mut self) {
        self.segments = SegmentMap::default();
        self.total = None;
        self.resumable = None;
        self.etag = None;
        self.last_modified = None;
        self.parts_done = 0;
        self.parts_total = 0;
        self.stream_bytes = 0;
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Schedule {
    pub enabled: bool,
    /// Minutes after local midnight.
    pub start_min: u32,
    pub stop_min: u32,
    /// Monday first.
    pub days: [bool; 7],
}

impl Default for Schedule {
    fn default() -> Self {
        Self { enabled: false, start_min: 60, stop_min: 7 * 60, days: [true; 7] }
    }
}

impl Schedule {
    /// `weekday` is 0 for Monday. A window may wrap past midnight, in which case
    /// the early hours belong to the previous day.
    pub fn allows(&self, weekday: usize, minute: u32) -> bool {
        if !self.enabled {
            return true;
        }
        let (a, b) = (self.start_min, self.stop_min);
        if a == b {
            self.days[weekday]
        } else if a < b {
            self.days[weekday] && minute >= a && minute < b
        } else if minute >= a {
            self.days[weekday]
        } else {
            minute < b && self.days[(weekday + 6) % 7]
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub download_dir: PathBuf,
    pub category_folders: bool,
    pub max_connections: usize,
    pub max_active: usize,
    /// Bytes per second across all downloads, 0 = unlimited.
    pub speed_limit: u64,
    pub max_retries: u32,
    /// Smallest piece worth an extra connection.
    pub min_split: u64,
    /// `http://`, `https://` or `socks5://`; empty uses the system proxy.
    pub proxy: String,
    pub user_agent: String,
    pub schedule: Schedule,
    /// Show a confirmation window for downloads sent by the browser.
    pub confirm_browser: bool,
    /// Offer to download a file address copied to the clipboard.
    pub watch_clipboard: bool,
    /// System notifications when downloads finish or fail.
    pub notify_complete: bool,
    pub notify_failed: bool,
    pub notify_sound: bool,
}

impl Default for Settings {
    fn default() -> Self {
        let download_dir = dirs::download_dir()
            .or_else(dirs::home_dir)
            .unwrap_or_else(|| PathBuf::from("."))
            .join("FluxDM");
        Self {
            download_dir,
            category_folders: true,
            max_connections: 16,
            max_active: 3,
            speed_limit: 0,
            max_retries: 8,
            min_split: 256 * 1024,
            proxy: String::new(),
            user_agent: concat!("FluxDM/", env!("CARGO_PKG_VERSION")).to_string(),
            schedule: Schedule::default(),
            confirm_browser: true,
            watch_clipboard: true,
            notify_complete: true,
            notify_failed: true,
            notify_sound: true,
        }
    }
}

pub const MAX_CONNECTIONS: usize = 32;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn overnight_window_belongs_to_the_starting_day() {
        let mut s = Schedule { enabled: true, start_min: 23 * 60, stop_min: 6 * 60, days: [false; 7] };
        s.days[0] = true; // Monday
        assert!(s.allows(0, 23 * 60 + 30));
        assert!(s.allows(1, 60)); // early Tuesday
        assert!(!s.allows(0, 60));
        assert!(!s.allows(1, 23 * 60 + 30));
    }
}
