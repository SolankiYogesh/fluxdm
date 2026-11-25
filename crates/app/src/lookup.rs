//! Fetches a file's details in the background while a dialog is open.

use crate::theme::{self, icon};
use crate::format;
use eframe::egui::{self, RichText};
use fluxdm_core::preview::{self, FileInfo};
use fluxdm_core::{naming, DownloadRequest, Manager};
use std::sync::mpsc;
use std::time::{Duration, Instant};

/// Wait for typing to stop before contacting a server.
const SETTLE: Duration = Duration::from_millis(500);

#[derive(Default)]
pub struct Lookup {
    key: Option<String>,
    changed: Option<Instant>,
    pending: Option<mpsc::Receiver<Result<FileInfo, String>>>,
    result: Option<Result<FileInfo, String>>,
}

impl Lookup {
    pub fn info(&self) -> Option<&FileInfo> {
        self.result.as_ref()?.as_ref().ok()
    }

    pub fn failure(&self) -> Option<&str> {
        self.result.as_ref()?.as_ref().err().map(String::as_str)
    }

    pub fn is_busy(&self) -> bool {
        self.key.is_some() && self.result.is_none()
    }

    /// Call every frame with the request being shown, or `None`.
    pub fn update(
        &mut self,
        ctx: &egui::Context,
        manager: &Manager,
        runtime: &tokio::runtime::Handle,
        request: Option<DownloadRequest>,
    ) {
        let request = request.filter(|r| preview::is_fetchable(&r.url));
        // Anything that changes what the server would answer.
        let key = request.as_ref().map(|r| {
            [Some(&r.url), r.referrer.as_ref(), r.cookie.as_ref(), r.username.as_ref(), r.password.as_ref()]
                .map(|part| part.map_or("", String::as_str))
                .join("\n")
        });
        if key != self.key {
            // Dropping the receiver discards an answer for the old address.
            *self = Self { changed: key.is_some().then(Instant::now), key, ..Default::default() };
        }
        if let Some(waited) = self.changed.map(|t| t.elapsed()) {
            if waited < SETTLE {
                ctx.request_repaint_after(SETTLE - waited);
            } else if let Some(request) = request {
                self.changed = None;
                let (send, receive) = mpsc::channel();
                self.pending = Some(receive);
                let (settings, ctx) = (manager.settings(), ctx.clone());
                runtime.spawn(async move {
                    let found = preview::inspect(&settings, &request).await.map_err(|e| format!("{e:#}"));
                    let _ = send.send(found);
                    ctx.request_repaint();
                });
            }
        }
        if let Some(answer) = self.pending.as_ref().and_then(|r| r.try_recv().ok()) {
            self.pending = None;
            self.result = Some(answer);
        }
    }

    /// The card under the address: name, size and where it will be saved.
    pub fn card(&self, ui: &mut egui::Ui, name: &str, folder: &std::path::Path) {
        theme::card().fill(theme::MUTED).inner_margin(egui::Margin::same(12)).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.spacing_mut().item_spacing.y = 5.0;
            ui.horizontal(|ui| {
                ui.label(theme::glyph(icon::for_category(naming::category(name)), 22.0, theme::BLUE));
                ui.add(egui::Label::new(RichText::new(name).font(theme::semibold(14.0))).truncate());
            });
            ui.horizontal(|ui| {
                if self.is_busy() {
                    ui.add(egui::Spinner::new().size(14.0));
                    ui.label(theme::muted("Fetching file details…"));
                    return;
                }
                if let Some(info) = self.info() {
                    let size = match info.size {
                        Some(n) => format::bytes(n),
                        None if info.is_stream => "Video stream, size not known yet".into(),
                        None => "Size not given by server".into(),
                    };
                    theme::badge(ui, &size, theme::FOREGROUND);
                    theme::badge(ui, naming::category(name), theme::BLUE);
                    match info.resumable {
                        Some(true) => theme::badge(ui, "Resumable", theme::GREEN),
                        Some(false) => theme::badge(ui, "Cannot resume", theme::AMBER),
                        None => {}
                    }
                } else {
                    theme::badge(ui, naming::category(name), theme::BLUE);
                }
            });
            ui.horizontal(|ui| {
                ui.label(theme::glyph(icon::FOLDER, 13.0, theme::MUTED_FG));
                let shown = folder.display().to_string();
                ui.add(egui::Label::new(theme::muted(format!("Saves to {shown}"))).truncate()).on_hover_text(shown);
            });
            let free = preview::free_name(folder, name);
            if free != name {
                ui.label(RichText::new(format!("That file exists, so it will be saved as {free}")).color(theme::AMBER));
            }
            if self.info().is_some_and(|i| i.is_web_page) {
                ui.label(RichText::new("This address is a web page, not a file.").color(theme::AMBER));
            }
            if let Some(why) = self.failure() {
                ui.add(
                    egui::Label::new(
                        RichText::new(format!("Could not fetch details: {why}. You can still try to download."))
                            .color(theme::AMBER),
                    )
                    .wrap(),
                );
            }
        });
    }
}
