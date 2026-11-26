//! Tells the user when downloads finish: system notifications, plus cards inside
//! the window.

use crate::theme::{self, icon, Kind};
use eframe::egui::{self, RichText};
use fluxdm_core::events::{summarize, Message};
use fluxdm_core::{Event, Manager};
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

/// Downloads finishing close together share one notification: send after this
/// much quiet...
const QUIET: Duration = Duration::from_millis(1500);
/// ...but never hold the first one back longer than this.
const LONGEST_WAIT: Duration = Duration::from_secs(6);

const CARD_LIFE: Duration = Duration::from_secs(10);
const CARDS_SHOWN: usize = 3;
const CARDS_KEPT: usize = 20;

struct Card {
    event: Event,
    /// Set when first drawn, so a card does not expire while the window is hidden.
    shown: Option<Instant>,
}

#[derive(Default)]
pub struct Center {
    cards: Mutex<Vec<Card>>,
    /// Why the last system notification could not be shown.
    problem: Mutex<Option<String>>,
}

#[cfg(target_os = "windows")]
const APP_ID: &str = "FluxDM";

/// Windows shows a notification under an app's name only once that name has been
/// registered for the user.
#[cfg(target_os = "windows")]
fn register_app_id() {
    use std::os::windows::process::CommandExt;
    const NO_WINDOW: u32 = 0x0800_0000;
    let key = format!(r"HKCU\Software\Classes\AppUserModelId\{APP_ID}");
    let _ = std::process::Command::new("reg")
        .args(["add", &key, "/v", "DisplayName", "/t", "REG_SZ", "/d", "FluxDM", "/f"])
        .creation_flags(NO_WINDOW)
        .status();
}

/// Ask the operating system for permission or registration. May block while the
/// user answers, so call it off the window thread.
fn prepare() -> Result<(), String> {
    #[cfg(target_os = "macos")]
    match notify_rust::request_auth_blocking() {
        Ok(true) => {}
        Ok(false) => {
            return Err("macOS has not allowed them. Check System Settings > Notifications > FluxDM.".into())
        }
        Err(e) => return Err(e.to_string()),
    }
    #[cfg(target_os = "windows")]
    register_app_id();
    Ok(())
}

fn show_system(message: &Message, sound: bool) -> Result<(), String> {
    let mut note = notify_rust::Notification::new();
    note.summary(&message.title).body(&message.body);
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        note.appname("FluxDM").icon("fluxdm");
        if sound {
            note.sound_name("message-new-instant");
        }
    }
    #[cfg(target_os = "macos")]
    if sound {
        // An empty name means the default sound.
        note.sound_name("");
    }
    #[cfg(target_os = "windows")]
    {
        note.app_id(APP_ID);
        if sound {
            note.sound_name("Default");
        }
    }
    note.show().map(drop).map_err(|e| e.to_string())
}

impl Center {
    pub fn problem(&self) -> Option<String> {
        self.problem.lock().unwrap().clone()
    }

    fn record(&self, result: Result<(), String>) {
        if let Err(e) = &result {
            eprintln!("notification not shown: {e}");
        }
        *self.problem.lock().unwrap() = result.err();
    }

    /// Send a sample, for the button in Settings.
    pub fn test(self: &Arc<Self>, sound: bool, ctx: egui::Context) {
        let center = self.clone();
        std::thread::spawn(move || {
            let sample = Message { title: "FluxDM".into(), body: "Notifications are working.".into() };
            center.record(prepare().and_then(|()| show_system(&sample, sound)));
            ctx.request_repaint();
        });
    }

    fn add_cards(&self, events: &[Event]) {
        let mut cards = self.cards.lock().unwrap();
        cards.extend(events.iter().cloned().map(|event| Card { event, shown: None }));
        let extra = cards.len().saturating_sub(CARDS_KEPT);
        cards.drain(..extra);
    }

    /// A thread that shows system notifications. It is separate because asking
    /// for permission waits until the user answers, and nothing else may wait
    /// with it.
    fn sender(self: &Arc<Self>, manager: Arc<Manager>, ctx: egui::Context) -> mpsc::Sender<(Message, bool)> {
        let (send, receive) = mpsc::channel::<(Message, bool)>();
        let center = self.clone();
        std::thread::spawn(move || {
            // Ask early, before the first download finishes.
            let settings = manager.settings();
            if settings.notify_complete || settings.notify_failed {
                center.record(prepare());
                ctx.request_repaint();
            }
            for (message, sound) in receive {
                center.record(prepare().and_then(|()| show_system(&message, sound)));
                ctx.request_repaint();
            }
        });
        send
    }

    /// Collect finished downloads off the window thread: a hidden window may not
    /// be redrawn for a long time.
    pub fn watch(self: &Arc<Self>, manager: Arc<Manager>, ctx: egui::Context) {
        let center = self.clone();
        let system = self.sender(manager.clone(), ctx.clone());
        std::thread::spawn(move || {
            let mut batch: Vec<Event> = Vec::new();
            let (mut first, mut latest) = (Instant::now(), Instant::now());
            loop {
                std::thread::sleep(Duration::from_millis(300));
                let new = manager.take_events();
                if !new.is_empty() {
                    if batch.is_empty() {
                        first = Instant::now();
                    }
                    latest = Instant::now();
                    center.add_cards(&new);
                    batch.extend(new);
                    ctx.request_repaint();
                }
                if !batch.is_empty() && (latest.elapsed() >= QUIET || first.elapsed() >= LONGEST_WAIT) {
                    let s = manager.settings();
                    for message in summarize(&batch, s.notify_complete, s.notify_failed) {
                        // The sender only stops when the program does.
                        let _ = system.send((message, s.notify_sound));
                    }
                    batch.clear();
                }
            }
        });
    }

    /// Draw the newest cards in the lower right corner.
    pub fn show_cards(&self, ctx: &egui::Context) {
        let mut cards = self.cards.lock().unwrap();
        cards.retain(|c| c.shown.is_none_or(|t| t.elapsed() < CARD_LIFE));
        if cards.is_empty() {
            return;
        }
        let first = cards.len().saturating_sub(CARDS_SHOWN);
        let mut dismissed = None;
        egui::Area::new(egui::Id::unique("notification-cards"))
            .order(egui::Order::Tooltip)
            .anchor(egui::Align2::RIGHT_BOTTOM, [-20.0, -20.0])
            .show(ctx, |ui| {
                for (i, card) in cards.iter_mut().enumerate().skip(first) {
                    let since = *card.shown.get_or_insert_with(Instant::now);
                    ctx.request_repaint_after(CARD_LIFE.saturating_sub(since.elapsed()));
                    if toast(ui, &card.event) {
                        dismissed = Some(i);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(i) = dismissed {
            cards.remove(i);
        }
    }
}

/// One card. Returns true when it should go away.
fn toast(ui: &mut egui::Ui, event: &Event) -> bool {
    let mut done = false;
    let shadow =
        egui::Shadow { offset: [0, 10], blur: 28, spread: 0, color: egui::Color32::from_black_alpha(150) };
    theme::card().stroke(egui::Stroke::new(1.0, theme::INPUT)).shadow(shadow).show(ui, |ui| {
        ui.set_width(330.0);
        let (glyph, color, heading) = if event.failed() {
            (icon::CLOSE, theme::RED, "Download failed")
        } else {
            (icon::DONE, theme::GREEN, "Download complete")
        };
        ui.horizontal(|ui| {
            ui.label(theme::glyph(glyph, 16.0, color));
            ui.label(RichText::new(heading).font(theme::semibold(13.5)));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                done = theme::icon_button(ui, Kind::Ghost, icon::CLOSE, "Dismiss").clicked();
            });
        });
        ui.add(egui::Label::new(&event.name).truncate()).on_hover_text(&event.name);
        if let Some(why) = &event.error {
            ui.add(egui::Label::new(theme::muted(why)).wrap());
        }
        ui.horizontal(|ui| {
            if let Some(path) = &event.path {
                if theme::button(ui, Kind::Primary, icon::OPEN, "Open file").clicked() {
                    let _ = open::that_detached(path);
                    done = true;
                }
            }
            if let Some(dir) = event.dir.as_ref().filter(|d| d.is_dir()) {
                if theme::button(ui, Kind::Outline, icon::FOLDER_OPEN, "Show folder").clicked() {
                    let _ = open::that_detached(dir);
                }
            }
        });
    });
    done
}
