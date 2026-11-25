//! Offers to download a file address as soon as it is copied.

use fluxdm_core::{links, DownloadRequest, Manager};
use std::sync::Arc;
use std::time::Duration;

pub fn watch(manager: Arc<Manager>, on_offer: Arc<dyn Fn() + Send + Sync>) {
    std::thread::spawn(move || {
        let Ok(mut clipboard) = arboard::Clipboard::new() else { return };
        // `None` until first read, so text copied earlier is not acted on.
        let mut last: Option<String> = None;
        loop {
            std::thread::sleep(Duration::from_millis(900));
            if !manager.settings().watch_clipboard {
                last = None;
                continue;
            }
            let text = clipboard.get_text().unwrap_or_default();
            let Some(previous) = last.replace(text.clone()) else { continue };
            if previous == text {
                continue;
            }
            // Copying an address that is already listed is not a new request.
            let Some(url) = links::clipboard_candidate(&text) else { continue };
            if manager.list().iter().any(|v| v.task.req.url == url) {
                continue;
            }
            if manager.offer(DownloadRequest { url, ..Default::default() }).is_ok() {
                on_offer();
            }
        }
    });
}
