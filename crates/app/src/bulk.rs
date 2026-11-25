//! Bulk downloads: collect links from text, a file or a web page, then pick.

use crate::theme::{self, icon, Kind};
use eframe::egui::{self, RichText};
use fluxdm_core::links::{self, Link};
use fluxdm_core::{naming, DownloadRequest, Manager};
use std::sync::mpsc;

#[derive(Default)]
pub struct Bulk {
    text: String,
    page: String,
    referrer: Option<String>,
    rows: Vec<(bool, Link)>,
    filter: String,
    files_only: bool,
    status: String,
    grabbing: Option<mpsc::Receiver<Result<Vec<Link>, String>>>,
}

impl Bulk {
    pub fn new() -> Self {
        Self { files_only: true, ..Default::default() }
    }

    fn load(&mut self, found: Vec<Link>) {
        self.status = format!("Found {} link(s).", found.len());
        // Addresses the user typed are all wanted; on a page only files are.
        let from_page = self.referrer.is_some();
        self.files_only = from_page;
        self.rows = found.into_iter().map(|l| (!from_page || l.is_file, l)).collect();
    }

    fn visible(&self, link: &Link) -> bool {
        let needle = self.filter.to_lowercase();
        (!self.files_only || link.is_file)
            && needle.split_whitespace().all(|w| link.url.to_lowercase().contains(w))
    }

    fn download(&mut self, manager: &Manager, start: bool) -> bool {
        let chosen: Vec<Link> = self
            .rows
            .iter()
            .filter(|(on, l)| *on && self.visible(l))
            .map(|(_, l)| l.clone())
            .collect();
        let mut failed = 0;
        for link in &chosen {
            let req = DownloadRequest {
                url: link.url.clone(),
                referrer: self.referrer.clone(),
                ..Default::default()
            };
            failed += usize::from(manager.add(req, start).is_err());
        }
        if failed > 0 {
            self.status = format!("{failed} link(s) could not be added.");
        }
        failed == 0 && !chosen.is_empty()
    }

    /// Returns false once the window should close.
    pub fn show(&mut self, ctx: &egui::Context, manager: &Manager, runtime: &tokio::runtime::Handle) -> bool {
        if let Some(result) = self.grabbing.as_ref().and_then(|r| r.try_recv().ok()) {
            self.grabbing = None;
            match result {
                Ok(found) => self.load(found),
                Err(e) => self.status = e,
            }
        }
        let mut close = false;
        let about = "Collect links from a web page, pasted text or a file, then tick the ones you want.";
        let dismissed = theme::dialog(ctx, "bulk", "Bulk download", about, 720.0, |ui| {
            theme::field(ui, "Web page", |ui| {
                ui.horizontal(|ui| {
                    let width = (ui.available_width() - 142.0).max(200.0);
                    ui.add(theme::input(&mut self.page, "https://").desired_width(width));
                    let ready = self.grabbing.is_none() && !self.page.trim().is_empty();
                    ui.add_enabled_ui(ready, |ui| {
                        if theme::button(ui, Kind::Outline, icon::LINK, "Grab links").clicked() {
                            let (send, receive) = mpsc::channel();
                            let (settings, page, ctx) = (manager.settings(), self.page.trim().to_string(), ui.ctx().clone());
                            self.referrer = Some(page.clone());
                            self.rows.clear();
                            self.status = "Reading the page…".into();
                            self.grabbing = Some(receive);
                            runtime.spawn(async move {
                                let found = links::grab_page(&settings, &page).await.map_err(|e| format!("{e:#}"));
                                let _ = send.send(found);
                                ctx.request_repaint();
                            });
                        }
                    });
                });
            });
            theme::field(ui, "Or paste text containing links  (file[001-100].jpg makes a series)", |ui| {
                egui::ScrollArea::vertical().id_salt("paste").max_height(64.0).show(ui, |ui| {
                    ui.add(
                        egui::TextEdit::multiline(&mut self.text)
                            .margin(egui::Margin::symmetric(10, 8))
                            .desired_rows(2)
                            .desired_width(f32::INFINITY),
                    );
                });
            });
            ui.horizontal(|ui| {
                if theme::button(ui, Kind::Outline, icon::SEARCH, "Find links in text").clicked() {
                    self.referrer = None;
                    self.load(links::from_text(&self.text));
                }
                if theme::button(ui, Kind::Ghost, icon::DOCUMENT, "Import file").clicked() {
                    if let Some(path) = rfd::FileDialog::new().pick_file() {
                        let small = std::fs::metadata(&path).is_ok_and(|m| m.len() < 8 << 20);
                        match std::fs::read(&path) {
                            Ok(data) if small => {
                                self.referrer = None;
                                self.load(links::from_text(&String::from_utf8_lossy(&data)));
                            }
                            _ => self.status = "Could not read that file (limit 8 MB).".into(),
                        }
                    }
                }
            });
            ui.separator();
            ui.horizontal(|ui| {
                ui.add(theme::input(&mut self.filter, "Filter, e.g. .mp3").desired_width(200.0));
                if theme::chip(ui, self.files_only, "Files only").clicked() {
                    self.files_only = !self.files_only;
                }
                let mut set = None;
                if theme::button(ui, Kind::Ghost, "", "Select all").clicked() {
                    set = Some(true);
                }
                if theme::button(ui, Kind::Ghost, "", "Select none").clicked() {
                    set = Some(false);
                }
                if let Some(value) = set {
                    for i in 0..self.rows.len() {
                        if self.visible(&self.rows[i].1) {
                            self.rows[i].0 = value;
                        }
                    }
                }
            });
            if !self.rows.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(theme::muted("Tick only:"));
                    for category in naming::CATEGORIES {
                        if self.rows.iter().any(|(_, l)| l.category == category)
                            && theme::button(ui, Kind::Outline, icon::for_category(category), category).clicked()
                        {
                            for row in &mut self.rows {
                                row.0 = row.1.category == category;
                            }
                        }
                    }
                });
            }
            let shown: Vec<usize> = (0..self.rows.len()).filter(|&i| self.visible(&self.rows[i].1)).collect();
            theme::card().fill(theme::BACKGROUND).inner_margin(egui::Margin::same(8)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                let height = (ui.ctx().content_rect().height() - 580.0).clamp(96.0, 230.0);
                ui.set_min_height(height);
                if shown.is_empty() {
                    ui.vertical_centered(|ui| {
                        ui.add_space(height / 2.0 - 12.0);
                        ui.label(theme::muted("Links you find appear here."));
                    });
                    return;
                }
                egui::ScrollArea::vertical().id_salt("found").max_height(height).auto_shrink([false, false]).show_rows(
                    ui,
                    24.0,
                    shown.len(),
                    |ui, range| {
                        for &i in &shown[range] {
                            let (on, link) = &mut self.rows[i];
                            ui.horizontal(|ui| {
                                ui.set_height(24.0);
                                ui.checkbox(on, "");
                                ui.label(theme::glyph(icon::for_category(link.category), 14.0, theme::MUTED_FG));
                                ui.label(RichText::new(&link.name).font(theme::medium(12.5))).on_hover_text(&link.url);
                                ui.add(egui::Label::new(theme::muted(&link.url)).truncate());
                            });
                        }
                    },
                );
            });
            let count = shown.iter().filter(|&&i| self.rows[i].0).count();
            ui.label(theme::muted(format!("{}  {count} selected.", self.status)));
            ui.horizontal(|ui| {
                ui.add_enabled_ui(count > 0, |ui| {
                    let label = format!("Download {count} now");
                    if theme::button(ui, Kind::Primary, icon::DOWNLOAD, &label).clicked() {
                        close |= self.download(manager, true);
                    }
                    if theme::button(ui, Kind::Outline, icon::CLOCK, "Add paused").clicked() {
                        close |= self.download(manager, false);
                    }
                });
                close |= theme::button(ui, Kind::Ghost, "", "Cancel").clicked();
            });
        });
        !(close || dismissed)
    }
}
