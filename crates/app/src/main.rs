//! FluxDM desktop window.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod bulk;
mod clipboard;
mod dialogs;
mod format;
mod lookup;
mod notify;
mod theme;
mod views;

use dialogs::{AddForm, DeleteConfirm, Options};
use eframe::egui;
use fluxdm_core::ipc::{self, Command};
use fluxdm_core::manager::default_data_dir;
use fluxdm_core::{Manager, Status, Task};
use std::collections::BTreeSet;
use std::sync::Arc;
use std::time::Duration;

#[derive(Clone, Copy, PartialEq)]
enum Filter {
    All,
    Unfinished,
    Finished,
    Category(&'static str),
}

impl Filter {
    fn matches(self, t: &Task) -> bool {
        match self {
            Filter::All => true,
            Filter::Unfinished => t.status != Status::Completed,
            Filter::Finished => t.status == Status::Completed,
            Filter::Category(c) => t.category == c,
        }
    }
}

struct App {
    manager: Arc<Manager>,
    filter: Filter,
    search: String,
    selected: BTreeSet<u64>,
    add: Option<AddForm>,
    bulk: Option<bulk::Bulk>,
    runtime: tokio::runtime::Handle,
    options: Option<Options>,
    delete: Option<DeleteConfirm>,
    new_url: String,
    message: String,
    logo: egui::TextureHandle,
    offer_lookup: lookup::Lookup,
    notices: Arc<notify::Center>,
}

impl eframe::App for App {
    fn logic(&mut self, ctx: &egui::Context, _: &mut eframe::Frame) {
        ctx.request_repaint_after(Duration::from_millis(300));
    }

    fn ui(&mut self, ui: &mut egui::Ui, _: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();
        let views = self.manager.list();
        self.selected.retain(|id| views.iter().any(|v| v.task.id == *id));

        self.layout(ui, &views);

        if self.add.as_mut().is_some_and(|f| !f.show(&ctx, &self.manager, &self.runtime)) {
            self.add = None;
        }
        if self.bulk.as_mut().is_some_and(|f| !f.show(&ctx, &self.manager, &self.runtime)) {
            self.bulk = None;
        }
        if self.options.as_mut().is_some_and(|f| !f.show(&ctx, &self.manager, &self.notices)) {
            self.options = None;
        }
        if self.delete.as_mut().is_some_and(|f| !f.show(&ctx, &self.manager)) {
            self.delete = None;
        }
        if self.add.is_none() && self.bulk.is_none() && self.options.is_none() && self.delete.is_none() {
            dialogs::confirm_browser(&ctx, &self.manager, &self.runtime, &mut self.offer_lookup);
        }
        // Cards wait until no dialog is open, so they never cover its buttons.
        let dialog_open = self.add.is_some()
            || self.bulk.is_some()
            || self.options.is_some()
            || self.delete.is_some()
            || self.manager.pending().is_some();
        if !dialog_open {
            self.notices.show_cards(&ctx);
        }
    }

    fn on_exit(&mut self) {
        self.manager.shutdown();
    }
}

fn main() -> anyhow::Result<()> {
    let data_dir = default_data_dir();
    if ipc::request(&data_dir, Command::Ping).is_ok_and(|r| r.ok) {
        eprintln!("FluxDM is already running.");
        return Ok(());
    }
    let runtime = tokio::runtime::Runtime::new()?;
    let manager = Manager::open(data_dir, runtime.handle().clone())?;

    // Scheduling runs off the UI thread so a hidden window does not stall it.
    let ticker = manager.clone();
    runtime.spawn(async move {
        loop {
            ticker.tick();
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    });

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("FluxDM")
            .with_app_id("fluxdm")
            .with_icon(Arc::new(eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))?))
            .with_inner_size([1180.0, 720.0])
            .with_min_inner_size([900.0, 560.0]),
        ..Default::default()
    };
    let handle = runtime.handle().clone();
    eframe::run_native(
        "FluxDM",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let ctx = cc.egui_ctx.clone();
            let notify = Arc::new(move || {
                ctx.send_viewport_cmd(egui::ViewportCommand::Minimized(false));
                ctx.send_viewport_cmd(egui::ViewportCommand::Focus);
                ctx.request_repaint();
            });
            handle.block_on(ipc::serve(manager.clone(), notify.clone()))?;
            clipboard::watch(manager.clone(), notify);
            let notices = Arc::new(notify::Center::default());
            notices.watch(manager.clone(), cc.egui_ctx.clone());
            let icon = eframe::icon_data::from_png_bytes(include_bytes!("../assets/icon.png"))?;
            let logo = cc.egui_ctx.load_texture(
                "logo",
                egui::ColorImage::from_rgba_unmultiplied([icon.width as usize, icon.height as usize], &icon.rgba),
                egui::TextureOptions { mipmap_mode: Some(egui::TextureFilter::Linear), ..egui::TextureOptions::LINEAR },
            );
            Ok(Box::new(App {
                logo,
                offer_lookup: Default::default(),
                notices,
                manager,
                filter: Filter::All,
                search: String::new(),
                selected: BTreeSet::new(),
                add: None,
                bulk: None,
                runtime: handle.clone(),
                options: None,
                delete: None,
                new_url: String::new(),
                message: String::new(),
            }))
        }),
    )
    .map_err(|e| anyhow::anyhow!("window system error: {e}"))
}
