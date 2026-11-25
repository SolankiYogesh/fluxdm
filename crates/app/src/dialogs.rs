//! Add, confirm, options and delete dialogs.

use crate::theme::{self, icon, Kind};
use eframe::egui::{self, RichText};
use fluxdm_core::types::MAX_CONNECTIONS;
use fluxdm_core::{naming, DownloadRequest, Manager, Settings};
use std::path::PathBuf;

#[derive(Default)]
pub struct AddForm {
    pub urls: String,
    name: String,
    folder: String,
    connections: usize,
    username: String,
    password: String,
    referrer: String,
    checksum: String,
    error: String,
}

fn optional(s: &str) -> Option<String> {
    (!s.trim().is_empty()).then(|| s.trim().to_string())
}

fn folder_row(ui: &mut egui::Ui, value: &mut String, hint: &str) {
    ui.horizontal(|ui| {
        let width = (ui.available_width() - 118.0).max(160.0);
        ui.add(theme::input(value, hint).desired_width(width));
        if theme::button(ui, Kind::Outline, icon::FOLDER, "Browse").clicked() {
            if let Some(p) = rfd::FileDialog::new().pick_folder() {
                *value = p.display().to_string();
            }
        }
    });
}

impl AddForm {
    fn requests(&self) -> Result<Vec<DownloadRequest>, String> {
        let mut urls = Vec::new();
        for line in self.urls.lines().map(str::trim).filter(|l| !l.is_empty()) {
            urls.extend(naming::expand_batch(line)?);
        }
        if urls.is_empty() {
            return Err("Enter at least one address.".into());
        }
        let single = urls.len() == 1;
        Ok(urls
            .into_iter()
            .map(|url| DownloadRequest {
                url,
                filename: optional(&self.name).filter(|_| single),
                dir: optional(&self.folder).map(PathBuf::from),
                connections: (self.connections > 0).then_some(self.connections),
                username: optional(&self.username),
                password: (!self.password.is_empty()).then(|| self.password.clone()),
                referrer: optional(&self.referrer),
                checksum: optional(&self.checksum).filter(|_| single),
                ..Default::default()
            })
            .collect())
    }

    /// Returns false once the window should close.
    pub fn show(&mut self, ctx: &egui::Context, manager: &Manager) -> bool {
        let mut close = false;
        let about = "One address per line. A pattern like file[01-20].zip adds a batch.";
        let dismissed = theme::dialog(ctx, "add", "Add download", about, 520.0, |ui| {
            ui.add(
                egui::TextEdit::multiline(&mut self.urls)
                    .hint_text("https://")
                    .margin(egui::Margin::symmetric(10, 8))
                    .desired_rows(3)
                    .desired_width(f32::INFINITY),
            );
            let line = |ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str| {
                theme::field(ui, label, |ui| ui.add(theme::input(value, hint).desired_width(f32::INFINITY)));
            };
            ui.columns(2, |c| {
                line(&mut c[0], "File name", &mut self.name, "Automatic");
                theme::field(&mut c[1], "Connections (0 = default)", |ui| {
                    ui.add(egui::Slider::new(&mut self.connections, 0..=MAX_CONNECTIONS));
                });
            });
            theme::field(ui, "Save to", |ui| folder_row(ui, &mut self.folder, "Category folder"));
            ui.collapsing("Sign-in, referrer and checksum", |ui| {
                ui.columns(2, |c| {
                    line(&mut c[0], "User name", &mut self.username, "");
                    theme::field(&mut c[1], "Password", |ui| {
                        ui.add(theme::input(&mut self.password, "").password(true).desired_width(f32::INFINITY))
                    });
                });
                line(ui, "Referrer page", &mut self.referrer, "");
                line(ui, "Checksum", &mut self.checksum, "sha256:…  (optional)");
            });
            if !self.error.is_empty() {
                ui.colored_label(theme::RED, &self.error);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let now = theme::button(ui, Kind::Primary, icon::DOWNLOAD, "Start download").clicked();
                let later = theme::button(ui, Kind::Outline, icon::CLOCK, "Download later").clicked();
                if now || later {
                    match self.requests() {
                        Ok(list) => {
                            self.error.clear();
                            for r in list {
                                match manager.add(r, now) {
                                    Ok(_) => close = true,
                                    Err(e) => self.error = e.to_string(),
                                }
                            }
                            close &= self.error.is_empty();
                        }
                        Err(e) => self.error = e,
                    }
                }
                close |= theme::button(ui, Kind::Ghost, "", "Cancel").clicked();
            });
        });
        !(close || dismissed)
    }
}

/// Ask before taking a download sent by the browser.
pub fn confirm_browser(ctx: &egui::Context, manager: &Manager) {
    let Some((req, waiting)) = manager.pending() else { return };
    let name = naming::guess_name(req.filename.as_deref(), None, &req.url);
    let dismissed = theme::dialog(ctx, "browser", "Download from browser", "", 500.0, |ui| {
        theme::card().fill(theme::MUTED).show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(theme::glyph(icon::for_category(naming::category(&name)), 22.0, theme::BLUE));
                ui.vertical(|ui| {
                    ui.add(egui::Label::new(RichText::new(&name).font(theme::semibold(14.0))).truncate());
                    ui.add(egui::Label::new(theme::muted(&req.url)).truncate());
                });
            });
        });
        ui.horizontal(|ui| {
            theme::badge(ui, naming::category(&name), theme::BLUE);
            if waiting > 1 {
                ui.label(theme::muted(format!("{} more waiting", waiting - 1)));
            }
        });
        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let now = theme::button(ui, Kind::Primary, icon::DOWNLOAD, "Start download").clicked();
            let later = theme::button(ui, Kind::Outline, icon::CLOCK, "Later").clicked();
            let all = waiting > 1 && theme::button(ui, Kind::Outline, icon::LAYERS, "Start all").clicked();
            let skip = theme::button(ui, Kind::Ghost, "", "Cancel").clicked();
            if now || later {
                if let Some(r) = manager.take_pending() {
                    let _ = manager.add(r, now);
                }
            } else if skip {
                manager.take_pending();
            } else if all {
                while let Some(r) = manager.take_pending() {
                    let _ = manager.add(r, true);
                }
            }
        });
    });
    if dismissed {
        manager.take_pending();
    }
}

pub struct Options {
    draft: Settings,
    folder: String,
    limit_kb: u64,
    error: String,
}

fn time_of_day(ui: &mut egui::Ui, minutes: &mut u32) {
    let (mut h, mut m) = (*minutes / 60, *minutes % 60);
    ui.horizontal(|ui| {
        ui.add(egui::DragValue::new(&mut h).range(0..=23));
        ui.label(":");
        ui.add(egui::DragValue::new(&mut m).range(0..=59).custom_formatter(|v, _| format!("{v:02}")));
    });
    *minutes = h * 60 + m;
}

impl Options {
    pub fn new(settings: Settings) -> Self {
        Self {
            folder: settings.download_dir.display().to_string(),
            limit_kb: settings.speed_limit / 1024,
            draft: settings,
            error: String::new(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, manager: &Manager) -> bool {
        let mut close = false;
        let dismissed = theme::dialog(ctx, "options", "Settings", "Applies to new and queued downloads.", 560.0, |ui| {
            let s = &mut self.draft;
            // The fields scroll; Save and Cancel below stay in view.
            let height = (ui.ctx().content_rect().height() - 290.0).max(160.0);
            egui::ScrollArea::vertical().id_salt("fields").max_height(height).min_scrolled_height(height).show(ui, |ui| {
            theme::field(ui, "Download folder", |ui| folder_row(ui, &mut self.folder, ""));
            ui.columns(2, |c| {
                theme::field(&mut c[0], "Connections per download", |ui| {
                    ui.add(egui::Slider::new(&mut s.max_connections, 1..=MAX_CONNECTIONS));
                });
                theme::field(&mut c[1], "Downloads at the same time", |ui| {
                    ui.add(egui::Slider::new(&mut s.max_active, 1..=16));
                });
            });
            ui.columns(2, |c| {
                theme::field(&mut c[0], "Speed limit in KB/s (0 = none)", |ui| {
                    ui.add(egui::DragValue::new(&mut self.limit_kb).speed(10));
                });
                theme::field(&mut c[1], "Retries after errors", |ui| {
                    ui.add(egui::DragValue::new(&mut s.max_retries).range(0..=100));
                });
            });
            theme::field(ui, "Proxy", |ui| {
                ui.add(theme::input(&mut s.proxy, "System default, or socks5://host:port").desired_width(f32::INFINITY))
            });
            theme::field(ui, "User agent", |ui| ui.add(theme::input(&mut s.user_agent, "").desired_width(f32::INFINITY)));
            ui.add_space(4.0);
            theme::switch(ui, &mut s.category_folders, "Sort into category folders");
            theme::switch(ui, &mut s.watch_clipboard, "Offer to download file links I copy");
            theme::switch(ui, &mut s.confirm_browser, "Ask before browser downloads");
            ui.separator();
            ui.label(RichText::new("Scheduler").font(theme::semibold(14.0)));
            theme::switch(ui, &mut s.schedule.enabled, "Run the queue only during these hours");
            ui.add_enabled_ui(s.schedule.enabled, |ui| {
                ui.horizontal(|ui| {
                    ui.label("From");
                    time_of_day(ui, &mut s.schedule.start_min);
                    ui.label("to");
                    time_of_day(ui, &mut s.schedule.stop_min);
                });
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 4.0;
                    for (i, d) in ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"].iter().enumerate() {
                        if theme::chip(ui, s.schedule.days[i], d).clicked() {
                            s.schedule.days[i] = !s.schedule.days[i];
                        }
                    }
                });
            });
            ui.label(theme::muted("FluxDM must be running. Resume starts a download regardless of the schedule."));
            });
            ui.separator();
            if !self.error.is_empty() {
                ui.colored_label(theme::RED, &self.error);
            }
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if theme::button(ui, Kind::Primary, "", "Save changes").clicked() {
                    let mut s = self.draft.clone();
                    s.download_dir = PathBuf::from(self.folder.trim());
                    s.speed_limit = self.limit_kb.saturating_mul(1024);
                    if !s.download_dir.is_absolute() {
                        self.error = "Choose a full folder path.".into();
                    } else {
                        match manager.update_settings(s) {
                            Ok(()) => close = true,
                            Err(e) => self.error = format!("{e:#}"),
                        }
                    }
                }
                close |= theme::button(ui, Kind::Ghost, "", "Cancel").clicked();
            });
        });
        !(close || dismissed)
    }
}

pub struct DeleteConfirm {
    pub ids: Vec<u64>,
    pub delete_files: bool,
}

impl DeleteConfirm {
    pub fn show(&mut self, ctx: &egui::Context, manager: &Manager) -> bool {
        let mut keep = true;
        let about = format!("Remove {} download(s) from the list? Unfinished data is deleted.", self.ids.len());
        let dismissed = theme::dialog(ctx, "remove", "Remove downloads", &about, 420.0, |ui| {
            theme::switch(ui, &mut self.delete_files, "Also delete finished files from disk");
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if theme::button(ui, Kind::Destructive, icon::TRASH, "Remove").clicked() {
                    for id in &self.ids {
                        manager.remove(*id, self.delete_files);
                    }
                    keep = false;
                }
                keep &= !theme::button(ui, Kind::Ghost, "", "Cancel").clicked();
            });
        });
        keep && !dismissed
    }
}
