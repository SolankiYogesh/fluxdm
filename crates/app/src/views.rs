//! The main window: sidebar, header, download list and details.

use crate::theme::{self, icon, Kind};
use crate::{bulk, dialogs, format, App, Filter};
use dialogs::{AddForm, DeleteConfirm, Options};
use eframe::egui::{self, vec2, Color32, Margin, RichText, Stroke};
use egui_extras::{Column, TableBuilder};
use fluxdm_core::{Status, Task, TaskView};

fn status_badge(t: &Task) -> (&'static str, Color32) {
    match t.status {
        Status::Completed => ("Complete", theme::GREEN),
        Status::Failed => ("Error", theme::RED),
        Status::Queued => ("Queued", theme::MUTED_FG),
        Status::Paused => ("Paused", theme::AMBER),
        Status::Downloading => ("Downloading", theme::BLUE),
    }
}

/// One bar for the whole file with each segment's downloaded part filled in.
fn segment_bar(ui: &mut egui::Ui, t: &Task) {
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 14.0), egui::Sense::hover());
    let painter = ui.painter().with_clip_rect(rect);
    painter.rect_filled(rect, 4.0, theme::ACCENT);
    let fill = status_badge(t).1;
    let x = |f: f64| rect.left() + (f.clamp(0.0, 1.0) as f32) * rect.width();
    let span = |a: f64, b: f64| egui::Rect::from_x_y_ranges(x(a)..=x(b), rect.y_range());
    match t.total {
        _ if t.status == Status::Completed || t.parts_total > 0 => {
            painter.rect_filled(span(0.0, f64::from(t.fraction().unwrap_or(0.0))), 4.0, fill);
        }
        Some(total) if total > 0 => {
            let total = total as f64;
            for s in &t.segments.segs {
                let a = s.start as f64 / total;
                painter.rect_filled(span(a, s.position() as f64 / total), 0.0, fill);
                if s.start > 0 {
                    painter.vline(x(a), rect.y_range(), (1.0, theme::BACKGROUND));
                }
                if s.active {
                    painter.vline(x(s.position() as f64 / total), rect.y_range(), (2.0, theme::FOREGROUND));
                }
            }
        }
        _ => {}
    }
}

fn nav_item(ui: &mut egui::Ui, selected: bool, glyph: &str, label: &str, count: usize) -> bool {
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), 34.0), egui::Sense::click());
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, selected, label));
    let lit = selected || response.hovered() || response.has_focus();
    let color = if lit { theme::FOREGROUND } else { theme::MUTED_FG };
    let painter = ui.painter();
    if lit {
        painter.rect_filled(rect, f32::from(theme::RADIUS), if selected { theme::ACCENT } else { theme::MUTED });
    }
    if response.has_focus() {
        painter.rect_stroke(rect, 8.0, Stroke::new(2.0, theme::RING), egui::StrokeKind::Inside);
    }
    let y = rect.center().y;
    painter.text(egui::pos2(rect.left() + 11.0, y), egui::Align2::LEFT_CENTER, glyph, theme::icon_font(15.0), color);
    painter.text(egui::pos2(rect.left() + 36.0, y), egui::Align2::LEFT_CENTER, label, theme::medium(13.0), color);
    if count > 0 {
        painter.text(
            egui::pos2(rect.right() - 11.0, y),
            egui::Align2::RIGHT_CENTER,
            count.to_string(),
            theme::mono(11.5),
            theme::MUTED_FG,
        );
    }
    response.on_hover_cursor(egui::CursorIcon::PointingHand).clicked()
}

fn stat(ui: &mut egui::Ui, glyph: &str, label: &str, value: String, tint: Color32) {
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.label(theme::muted(label));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(theme::glyph(glyph, 15.0, tint));
            });
        });
        ui.label(RichText::new(value).font(theme::semibold(21.0)));
    });
}

impl App {
    pub fn layout(&mut self, ui: &mut egui::Ui, views: &[TaskView]) {
        let sidebar = egui::Frame::new().fill(theme::SIDEBAR).inner_margin(Margin::symmetric(12, 16));
        egui::Panel::left("sidebar")
            .exact_size(220.0)
            .resizable(false)
            .frame(sidebar)
            .show(ui, |ui| self.sidebar(ui, views));

        let header = egui::Frame::new().fill(theme::BACKGROUND).inner_margin(Margin { left: 20, right: 20, top: 16, bottom: 12 });
        egui::Panel::top("header").frame(header).show_separator_line(false).show(ui, |ui| {
            self.header(ui);
            ui.add_space(8.0);
            self.stats(ui, views);
        });

        if self.selected.len() == 1 {
            if let Some(v) = views.iter().find(|v| self.selected.contains(&v.task.id)) {
                let frame = egui::Frame::new().fill(theme::BACKGROUND).inner_margin(Margin { left: 20, right: 20, top: 0, bottom: 16 });
                egui::Panel::bottom("details").frame(frame).show_separator_line(false).resizable(false).show(ui, |ui| {
                    theme::card().show(ui, |ui| {
                        ui.set_width(ui.available_width());
                        self.details(ui, v);
                    });
                });
            }
        }

        let needle = self.search.to_lowercase();
        let mut rows: Vec<&TaskView> = views
            .iter()
            .filter(|v| self.filter.matches(&v.task))
            .filter(|v| v.task.file_name.to_lowercase().contains(&needle))
            .collect();
        rows.reverse();
        let body = egui::Frame::new().fill(theme::BACKGROUND).inner_margin(Margin { left: 20, right: 20, top: 0, bottom: 16 });
        egui::CentralPanel::default().frame(body).show(ui, |ui| {
            theme::card().inner_margin(Margin::same(12)).show(ui, |ui| {
                ui.set_min_size(ui.available_size());
                self.list_bar(ui, views, rows.len());
                ui.separator();
                if rows.is_empty() {
                    self.empty_state(ui, views.is_empty());
                } else {
                    self.table(ui, &rows);
                }
            });
        });
    }

    fn sidebar(&mut self, ui: &mut egui::Ui, views: &[TaskView]) {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            ui.add(egui::Image::new((self.logo.id(), vec2(34.0, 34.0))));
            ui.vertical(|ui| {
                ui.spacing_mut().item_spacing.y = 0.0;
                ui.label(RichText::new("FluxDM").font(theme::semibold(15.0)));
                ui.label(theme::muted("Download manager"));
            });
        });
        ui.add_space(16.0);
        let mut pick = None;
        let mut entry = |ui: &mut egui::Ui, current: Filter, filter: Filter, glyph: &str, label: &str| {
            let count = views.iter().filter(|v| filter.matches(&v.task)).count();
            if nav_item(ui, current == filter, glyph, label, count) {
                pick = Some(filter);
            }
        };
        let now = self.filter;
        entry(ui, now, Filter::All, icon::LAYERS, "All downloads");
        entry(ui, now, Filter::Unfinished, icon::CLOCK, "Unfinished");
        entry(ui, now, Filter::Finished, icon::DONE, "Finished");
        ui.add_space(12.0);
        ui.label(theme::muted("Categories"));
        for c in fluxdm_core::naming::CATEGORIES {
            entry(ui, now, Filter::Category(c), icon::for_category(c), c);
        }
        if let Some(filter) = pick {
            self.filter = filter;
            self.selected.clear();
        }
        ui.with_layout(egui::Layout::bottom_up(egui::Align::Min), |ui| {
            if nav_item(ui, false, icon::SETTINGS, "Settings", 0) {
                self.options = Some(Options::new(self.manager.settings()));
            }
        });
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            let heading = match self.filter {
                Filter::All => "All downloads",
                Filter::Unfinished => "Unfinished",
                Filter::Finished => "Finished",
                Filter::Category(c) => c,
            };
            ui.label(RichText::new(heading).font(theme::semibold(21.0)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, Kind::Primary, icon::PLUS, "Add URL").clicked() {
                    self.add.get_or_insert_with(AddForm::default);
                }
                if theme::button(ui, Kind::Outline, icon::LIST_PLUS, "Bulk / Grab").clicked() {
                    self.bulk.get_or_insert_with(bulk::Bulk::new);
                }
                let width = (ui.available_width() - 16.0).clamp(120.0, 240.0);
                ui.add(theme::input(&mut self.search, "Search downloads").desired_width(width));
                ui.label(theme::glyph(icon::SEARCH, 15.0, theme::MUTED_FG));
            });
        });
    }

    fn stats(&mut self, ui: &mut egui::Ui, views: &[TaskView]) {
        let speed: f64 = views.iter().map(|v| v.speed).sum();
        let count = |s: Status| views.iter().filter(|v| v.task.status == s).count();
        let done: u64 = views.iter().filter(|v| v.task.status == Status::Completed).map(|v| v.task.downloaded()).sum();
        ui.columns(4, |c| {
            stat(&mut c[0], icon::GAUGE, "Total speed", format!("{}/s", format::bytes(speed as u64)), theme::BLUE);
            stat(&mut c[1], icon::ACTIVITY, "Active", count(Status::Downloading).to_string(), theme::BLUE);
            stat(&mut c[2], icon::CLOCK, "Queued", count(Status::Queued).to_string(), theme::AMBER);
            stat(&mut c[3], icon::DONE, "Downloaded", format::bytes(done), theme::GREEN);
        });
    }

    fn list_bar(&mut self, ui: &mut egui::Ui, views: &[TaskView], shown: usize) {
        let chosen = || views.iter().filter(|v| self.selected.contains(&v.task.id));
        let any = !self.selected.is_empty();
        let can_resume = chosen().any(|v| matches!(v.task.status, Status::Paused | Status::Failed | Status::Queued));
        let can_pause = chosen().any(|v| matches!(v.task.status, Status::Downloading | Status::Queued));
        ui.horizontal(|ui| {
            if any {
                ui.label(RichText::new(format!("{} selected", self.selected.len())).font(theme::medium(13.0)));
            } else {
                ui.label(theme::muted(format!("{shown} item(s)")));
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if theme::button(ui, Kind::Ghost, icon::PAUSE, "Pause all").clicked() {
                    self.manager.pause_all();
                }
                if theme::button(ui, Kind::Ghost, icon::PLAY, "Start queue").clicked() {
                    self.manager.resume_all();
                }
                if any {
                    ui.separator();
                    if theme::icon_button(ui, Kind::Ghost, icon::TRASH, "Remove selected").clicked() {
                        self.delete = Some(DeleteConfirm { ids: self.selected.iter().copied().collect(), delete_files: false });
                    }
                    ui.add_enabled_ui(can_pause, |ui| {
                        if theme::button(ui, Kind::Outline, icon::PAUSE, "Pause").clicked() {
                            self.selected.iter().for_each(|&id| self.manager.pause(id));
                        }
                    });
                    ui.add_enabled_ui(can_resume, |ui| {
                        if theme::button(ui, Kind::Outline, icon::PLAY, "Resume").clicked() {
                            self.selected.iter().for_each(|&id| self.manager.resume(id));
                        }
                    });
                }
            });
        });
    }

    fn empty_state(&mut self, ui: &mut egui::Ui, nothing_at_all: bool) {
        ui.vertical_centered(|ui| {
            ui.add_space((ui.available_height() / 2.0 - 100.0).max(8.0));
            ui.label(theme::glyph(icon::INBOX, 40.0, theme::MUTED_FG));
            ui.label(theme::title(if nothing_at_all { "No downloads yet" } else { "Nothing matches" }));
            ui.label(theme::muted(if nothing_at_all {
                "Add an address, grab links in bulk, or send a download from your browser."
            } else {
                "Try another category or clear the search."
            }));
            ui.add_space(8.0);
            if nothing_at_all {
                if theme::wide_button(ui, Kind::Primary, icon::PLUS, "Add your first download", 220.0).clicked() {
                    self.add.get_or_insert_with(AddForm::default);
                }
            } else if theme::button(ui, Kind::Outline, "", "Show all downloads").clicked() {
                self.filter = Filter::All;
                self.search.clear();
            }
        });
    }

    fn table(&mut self, ui: &mut egui::Ui, rows: &[&TaskView]) {
        let selected = &mut self.selected;
        let manager = &self.manager;
        // Row highlight comes from the selection and hover colours.
        ui.visuals_mut().widgets.hovered.bg_fill = theme::MUTED;
        ui.visuals_mut().selection.bg_fill = theme::ACCENT;
        TableBuilder::new(ui)
            .sense(egui::Sense::click())
            .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
            .column(Column::remainder().at_least(220.0).clip(true))
            .column(Column::exact(84.0))
            .column(Column::exact(196.0).clip(true))
            .column(Column::exact(92.0))
            .column(Column::exact(70.0))
            .header(24.0, |mut h| {
                for title in ["Name", "Size", "Status", "Speed", "ETA"] {
                    h.col(|ui| {
                        ui.label(RichText::new(title).font(theme::medium(12.0)).color(theme::MUTED_FG));
                    });
                }
            })
            .body(|body| {
                body.rows(48.0, rows.len(), |mut row| {
                    let view = rows[row.index()];
                    let t = &view.task;
                    row.set_selected(selected.contains(&t.id));
                    let (label, color) = status_badge(t);
                    let line = |ui: &mut egui::Ui, text: RichText| {
                        ui.add(egui::Label::new(text).truncate().selectable(false));
                    };
                    let number = |s: String| RichText::new(s).font(theme::mono(12.0));
                    row.col(|ui| {
                        egui::Frame::new().fill(theme::MUTED).corner_radius(theme::RADIUS).inner_margin(Margin::same(8)).show(ui, |ui| {
                            ui.label(theme::glyph(icon::for_category(&t.category), 16.0, theme::FOREGROUND));
                        });
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 1.0;
                            ui.add_space(6.0);
                            line(ui, RichText::new(&t.file_name).font(theme::medium(13.0)));
                            let host = url::Url::parse(&t.req.url).ok().and_then(|u| u.host_str().map(str::to_string));
                            line(ui, theme::muted(format!("{}  ·  {}", host.unwrap_or_default(), format::date(t.added_at))));
                        });
                    });
                    row.col(|ui| line(ui, number(t.total.map_or_else(|| "—".into(), format::bytes))));
                    row.col(|ui| {
                        theme::badge(ui, label, color);
                        if t.status != Status::Completed {
                            if let Some(f) = t.fraction() {
                                ui.vertical(|ui| {
                                    ui.spacing_mut().item_spacing.y = 2.0;
                                    ui.add_space(9.0);
                                    line(ui, number(format!("{:.0}%", f * 100.0)));
                                    theme::progress(ui, f, color, ui.available_width().min(64.0));
                                });
                            }
                        }
                    });
                    row.col(|ui| {
                        if view.speed >= 1.0 {
                            line(ui, number(format!("{}/s", format::bytes(view.speed as u64))));
                        }
                    });
                    row.col(|ui| {
                        if let Some(total) = t.total.filter(|_| view.speed >= 1.0 && t.parts_total == 0) {
                            let left = total.saturating_sub(t.downloaded()) as f64 / view.speed;
                            line(ui, number(format::duration(left as u64)));
                        }
                    });
                    let response = row.response();
                    if response.clicked() {
                        let multi = response.ctx.input(|i| i.modifiers.command);
                        if !multi {
                            selected.clear();
                            selected.insert(t.id);
                        } else if !selected.remove(&t.id) {
                            selected.insert(t.id);
                        }
                    }
                    if response.double_clicked() {
                        match (t.status, t.final_path()) {
                            (Status::Completed, Some(p)) => {
                                let _ = open::that_detached(p);
                            }
                            (Status::Downloading | Status::Queued, _) => manager.pause(t.id),
                            _ => manager.resume(t.id),
                        }
                    }
                });
            });
    }

    fn open_path(&mut self, path: &std::path::Path) {
        if let Err(e) = open::that_detached(path) {
            self.message = format!("Could not open: {e}");
        }
    }

    fn details(&mut self, ui: &mut egui::Ui, view: &TaskView) {
        let t = &view.task;
        let (label, color) = status_badge(t);
        let mut close = false;
        ui.horizontal(|ui| {
            theme::badge(ui, label, color);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = theme::icon_button(ui, Kind::Ghost, icon::CLOSE, "Close details").clicked();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    ui.add(egui::Label::new(RichText::new(&t.file_name).font(theme::semibold(14.0))).truncate());
                });
            });
        });
        let size = t.total.map_or_else(|| "unknown size".into(), format::bytes);
        let mut facts = vec![format!("{} of {size}", format::bytes(t.downloaded()))];
        if t.status == Status::Downloading {
            facts.push(format!("{} connection(s)", t.segments.active_count().max(usize::from(t.parts_total > 0))));
        }
        if t.parts_total > 0 {
            facts.push(format!("{} / {} stream parts", t.parts_done, t.parts_total));
        }
        match t.resumable {
            Some(true) => facts.push("resumable".into()),
            Some(false) => facts.push("server cannot resume".into()),
            None => {}
        }
        ui.label(theme::muted(facts.join("  ·  ")));
        segment_bar(ui, t);
        ui.add(egui::Label::new(theme::muted(&t.req.url)).truncate());
        if let Some(dir) = &t.dir {
            ui.add(egui::Label::new(theme::muted(format!("Saved in {}", dir.display()))).truncate());
        }
        if let Some(e) = &t.error {
            ui.colored_label(theme::RED, e);
        }
        if let Some(n) = &t.note {
            ui.colored_label(theme::AMBER, n);
        }
        ui.horizontal(|ui| {
            if t.status == Status::Completed {
                if let Some(p) = t.final_path() {
                    if theme::button(ui, Kind::Primary, icon::OPEN, "Open file").clicked() {
                        self.open_path(&p);
                    }
                }
            }
            if let Some(dir) = &t.dir {
                if theme::button(ui, Kind::Outline, icon::FOLDER_OPEN, "Open folder").clicked() {
                    self.open_path(dir);
                }
            }
            if theme::button(ui, Kind::Outline, icon::COPY, "Copy address").clicked() {
                ui.ctx().copy_text(t.req.url.clone());
                self.message = "Address copied.".into();
            }
            if theme::button(ui, Kind::Outline, icon::RESTART, "Restart").on_hover_text("Discard progress and download again").clicked() {
                self.manager.restart(t.id);
            }
            ui.add_space(8.0);
            ui.label(theme::muted("Limit KB/s (0 = none)"));
            let mut kb = t.req.speed_limit / 1024;
            if ui.add(egui::DragValue::new(&mut kb).speed(10)).changed() {
                self.manager.set_speed_limit(t.id, kb.saturating_mul(1024));
            }
        });
        if matches!(t.status, Status::Paused | Status::Failed) {
            ui.horizontal(|ui| {
                ui.label(theme::muted("Link expired?"));
                ui.add(theme::input(&mut self.new_url, "Paste the new address").desired_width(330.0));
                if theme::button(ui, Kind::Outline, icon::LINK, "Update address").clicked() {
                    self.message = match self.manager.set_url(t.id, self.new_url.trim()) {
                        Ok(()) => {
                            self.new_url.clear();
                            "Address updated. Press Resume.".into()
                        }
                        Err(e) => e.to_string(),
                    };
                }
            });
        }
        if !self.message.is_empty() {
            ui.label(theme::muted(&self.message));
        }
        if close {
            self.selected.clear();
            self.message.clear();
        }
    }
}
