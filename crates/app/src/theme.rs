//! Dark design tokens (zinc scale, after shadcn/ui) and the shared components.

use eframe::egui::{self, vec2, Color32, CornerRadius, FontFamily, FontId, Margin, RichText, Stroke};
use std::sync::Arc;

pub const BACKGROUND: Color32 = Color32::from_rgb(9, 9, 11);
pub const SIDEBAR: Color32 = Color32::from_rgb(12, 12, 14);
pub const CARD: Color32 = Color32::from_rgb(17, 17, 20);
pub const MUTED: Color32 = Color32::from_rgb(24, 24, 27);
pub const ACCENT: Color32 = Color32::from_rgb(39, 39, 42);
pub const BORDER: Color32 = Color32::from_rgb(39, 39, 42);
pub const INPUT: Color32 = Color32::from_rgb(63, 63, 70);
pub const RING: Color32 = Color32::from_rgb(161, 161, 170);
pub const FOREGROUND: Color32 = Color32::from_rgb(250, 250, 250);
pub const MUTED_FG: Color32 = Color32::from_rgb(161, 161, 170);
pub const PRIMARY: Color32 = FOREGROUND;
pub const PRIMARY_FG: Color32 = Color32::from_rgb(24, 24, 27);
pub const BLUE: Color32 = Color32::from_rgb(96, 165, 250);
pub const GREEN: Color32 = Color32::from_rgb(74, 222, 128);
pub const AMBER: Color32 = Color32::from_rgb(251, 191, 36);
pub const RED: Color32 = Color32::from_rgb(248, 113, 113);

pub const RADIUS: u8 = 8;
pub const CARD_RADIUS: u8 = 12;
pub const CONTROL_HEIGHT: f32 = 34.0;

/// Lucide glyphs.
pub mod icon {
    pub const PLUS: &str = "\u{e13d}";
    pub const LIST_PLUS: &str = "\u{e23f}";
    pub const PLAY: &str = "\u{e13c}";
    pub const PAUSE: &str = "\u{e12e}";
    pub const TRASH: &str = "\u{e18e}";
    pub const SETTINGS: &str = "\u{e154}";
    pub const SEARCH: &str = "\u{e151}";
    pub const DOWNLOAD: &str = "\u{e0b2}";
    pub const FOLDER: &str = "\u{e0d7}";
    pub const FOLDER_OPEN: &str = "\u{e247}";
    pub const FILE: &str = "\u{e0c0}";
    pub const DOCUMENT: &str = "\u{e0cc}";
    pub const VIDEO: &str = "\u{e0d0}";
    pub const MUSIC: &str = "\u{e122}";
    pub const ARCHIVE: &str = "\u{e041}";
    pub const PROGRAM: &str = "\u{e426}";
    pub const LAYERS: &str = "\u{e529}";
    pub const DONE: &str = "\u{e226}";
    pub const CLOCK: &str = "\u{e087}";
    pub const GAUGE: &str = "\u{e1bf}";
    pub const CLOSE: &str = "\u{e1b2}";
    pub const COPY: &str = "\u{e09e}";
    pub const RESTART: &str = "\u{e148}";
    pub const OPEN: &str = "\u{e0b9}";
    pub const LINK: &str = "\u{e102}";
    pub const INBOX: &str = "\u{e0f7}";
    pub const ACTIVITY: &str = "\u{e038}";

    pub fn for_category(category: &str) -> &'static str {
        match category {
            "Video" => VIDEO,
            "Music" => MUSIC,
            "Documents" => DOCUMENT,
            "Compressed" => ARCHIVE,
            "Programs" => PROGRAM,
            _ => FILE,
        }
    }
}

fn family(name: &str) -> FontFamily {
    FontFamily::Name(name.into())
}

pub fn regular(size: f32) -> FontId {
    FontId::new(size, FontFamily::Proportional)
}

pub fn medium(size: f32) -> FontId {
    FontId::new(size, family("medium"))
}

pub fn semibold(size: f32) -> FontId {
    FontId::new(size, family("semibold"))
}

pub fn mono(size: f32) -> FontId {
    FontId::new(size, FontFamily::Monospace)
}

pub fn icon_font(size: f32) -> FontId {
    FontId::new(size, family("icons"))
}

pub fn glyph(icon: &str, size: f32, color: Color32) -> RichText {
    RichText::new(icon).font(icon_font(size)).color(color)
}

pub fn muted(text: impl Into<String>) -> RichText {
    RichText::new(text).font(regular(12.5)).color(MUTED_FG)
}

pub fn title(text: impl Into<String>) -> RichText {
    RichText::new(text).font(semibold(17.0)).color(FOREGROUND)
}

fn install_fonts(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    let files: [(&str, &[u8]); 5] = [
        ("geist", include_bytes!("../assets/Geist-Regular.ttf")),
        ("geist-medium", include_bytes!("../assets/Geist-Medium.ttf")),
        ("geist-semibold", include_bytes!("../assets/Geist-SemiBold.ttf")),
        ("geist-mono", include_bytes!("../assets/GeistMono-Regular.ttf")),
        ("lucide", include_bytes!("../assets/lucide.ttf")),
    ];
    for (name, bytes) in files {
        fonts.font_data.insert(name.into(), Arc::new(egui::FontData::from_static(bytes)));
    }
    // The built-in fonts stay behind ours so other scripts and symbols render.
    let fallback = fonts.families[&FontFamily::Proportional].clone();
    let with = |first: &str| {
        let mut list = vec![first.to_string()];
        list.extend(fallback.iter().cloned());
        list
    };
    fonts.families.insert(FontFamily::Proportional, with("geist"));
    fonts.families.insert(family("medium"), with("geist-medium"));
    fonts.families.insert(family("semibold"), with("geist-semibold"));
    fonts.families.entry(FontFamily::Monospace).or_default().insert(0, "geist-mono".into());
    fonts.families.insert(family("icons"), vec!["lucide".into()]);
    ctx.set_fonts(fonts);
}

pub fn install(ctx: &egui::Context) {
    install_fonts(ctx);
    ctx.set_theme(egui::Theme::Dark);
    ctx.all_styles_mut(|style| {
        use egui::TextStyle::*;
        style.text_styles = [
            (Small, regular(11.5)),
            (Body, regular(13.5)),
            (Button, medium(13.0)),
            (Heading, semibold(17.0)),
            (Monospace, mono(12.5)),
        ]
        .into();
        let s = &mut style.spacing;
        s.item_spacing = vec2(8.0, 8.0);
        s.button_padding = vec2(12.0, 6.0);
        s.interact_size.y = 30.0;
        s.icon_width = 16.0;
        s.icon_width_inner = 9.0;
        s.slider_rail_height = 5.0;
        s.slider_width = 190.0;
        s.menu_margin = Margin::same(6);
        s.scroll.bar_width = 8.0;
        style.interaction.selectable_labels = false;

        let v = &mut style.visuals;
        v.dark_mode = true;
        v.panel_fill = BACKGROUND;
        v.window_fill = CARD;
        v.window_stroke = Stroke::new(1.0, BORDER);
        v.window_corner_radius = CornerRadius::same(CARD_RADIUS);
        v.menu_corner_radius = CornerRadius::same(RADIUS);
        v.extreme_bg_color = BACKGROUND;
        v.text_edit_bg_color = Some(BACKGROUND);
        v.faint_bg_color = Color32::from_rgb(15, 15, 17);
        v.code_bg_color = MUTED;
        v.hyperlink_color = BLUE;
        v.warn_fg_color = AMBER;
        v.error_fg_color = RED;
        v.weak_text_color = Some(MUTED_FG);
        v.selection.bg_fill = Color32::from_rgb(40, 52, 74);
        v.selection.stroke = Stroke::new(1.0, FOREGROUND);
        v.slider_trailing_fill = true;
        v.interact_cursor = Some(egui::CursorIcon::PointingHand);

        let states = [
            (&mut v.widgets.noninteractive, CARD, BORDER, MUTED_FG),
            (&mut v.widgets.inactive, MUTED, INPUT, FOREGROUND),
            (&mut v.widgets.hovered, ACCENT, RING, FOREGROUND),
            (&mut v.widgets.active, INPUT, RING, FOREGROUND),
            (&mut v.widgets.open, ACCENT, INPUT, FOREGROUND),
        ];
        for (w, fill, stroke, text) in states {
            w.bg_fill = fill;
            w.weak_bg_fill = fill;
            w.bg_stroke = Stroke::new(1.0, stroke);
            w.fg_stroke = Stroke::new(1.0, text);
            w.corner_radius = CornerRadius::same(6);
            w.expansion = 0.0;
        }
        v.widgets.noninteractive.fg_stroke.color = FOREGROUND;
    });
}

#[derive(Clone, Copy, PartialEq)]
pub enum Kind {
    Primary,
    Outline,
    Ghost,
    Destructive,
}

fn mix(a: Color32, b: Color32, t: f32) -> Color32 {
    let f = |x: u8, y: u8| (f32::from(x) + (f32::from(y) - f32::from(x)) * t).round() as u8;
    Color32::from_rgba_premultiplied(f(a.r(), b.r()), f(a.g(), b.g()), f(a.b(), b.b()), f(a.a(), b.a()))
}

/// A button with an optional leading icon. With an empty label it is square and
/// `hint` names it for tooltips and screen readers.
fn button_raw(ui: &mut egui::Ui, kind: Kind, icon: &str, label: &str, hint: &str, width: f32) -> egui::Response {
    let enabled = ui.is_enabled();
    let (fill, hover, text, border) = match kind {
        Kind::Primary => (PRIMARY, Color32::from_rgb(212, 212, 216), PRIMARY_FG, PRIMARY),
        Kind::Outline => (BACKGROUND, ACCENT, FOREGROUND, BORDER),
        Kind::Ghost => (Color32::TRANSPARENT, ACCENT, FOREGROUND, Color32::TRANSPARENT),
        Kind::Destructive => {
            (Color32::from_rgb(153, 27, 27), Color32::from_rgb(127, 29, 29), FOREGROUND, Color32::TRANSPARENT)
        }
    };
    let text_galley = ui.painter().layout_no_wrap(label.to_string(), medium(13.0), text);
    let icon_galley = (!icon.is_empty()).then(|| ui.painter().layout_no_wrap(icon.to_string(), icon_font(15.0), text));
    let gap = if icon_galley.is_some() && !label.is_empty() { 7.0 } else { 0.0 };
    let content = text_galley.size().x + gap + icon_galley.as_ref().map_or(0.0, |g| g.size().x);
    let natural = if label.is_empty() { CONTROL_HEIGHT } else { content + 26.0 };
    let (rect, mut response) = ui.allocate_exact_size(vec2(natural.max(width), CONTROL_HEIGHT), egui::Sense::click());
    let name = if label.is_empty() { hint } else { label };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, enabled, name));
    if !hint.is_empty() {
        response = response.on_hover_text(hint);
    }
    if enabled {
        response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
    }
    if ui.is_rect_visible(rect) {
        let hovered = enabled && (response.hovered() || response.has_focus());
        let t = ui.ctx().animate_bool_with_time(response.id, hovered, 0.15);
        let mut background = mix(fill, hover, t);
        if response.is_pointer_button_down_on() {
            background = mix(background, BACKGROUND, 0.2);
        }
        let painter = ui.painter();
        painter.rect(
            rect,
            f32::from(RADIUS),
            background,
            Stroke::new(1.0, border),
            egui::StrokeKind::Inside,
        );
        if response.has_focus() {
            painter.rect_stroke(rect.expand(2.0), 10.0, Stroke::new(2.0, RING), egui::StrokeKind::Outside);
        }
        let mut x = rect.center().x - content / 2.0;
        if let Some(g) = icon_galley {
            let size = g.size();
            painter.galley_with_override_text_color(egui::pos2(x, rect.center().y - size.y / 2.0), g, text);
            x += size.x + gap;
        }
        let y = rect.center().y - text_galley.size().y / 2.0;
        painter.galley_with_override_text_color(egui::pos2(x, y), text_galley, text);
    }
    response
}

pub fn button(ui: &mut egui::Ui, kind: Kind, icon: &str, label: &str) -> egui::Response {
    button_raw(ui, kind, icon, label, "", 0.0)
}

pub fn icon_button(ui: &mut egui::Ui, kind: Kind, icon: &str, hint: &str) -> egui::Response {
    button_raw(ui, kind, icon, "", hint, 0.0)
}

pub fn wide_button(ui: &mut egui::Ui, kind: Kind, icon: &str, label: &str, width: f32) -> egui::Response {
    button_raw(ui, kind, icon, label, "", width)
}

/// A small rounded status label. Meaning is carried by the text, not only colour.
pub fn badge(ui: &mut egui::Ui, text: &str, color: Color32) {
    let galley = ui.painter().layout_no_wrap(text.to_string(), medium(11.5), color);
    let (rect, response) = ui.allocate_exact_size(vec2(galley.size().x + 18.0, 21.0), egui::Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Label, true, text));
    ui.painter().rect(
        rect,
        10.5,
        color.gamma_multiply(0.13),
        Stroke::new(1.0, color.gamma_multiply(0.35)),
        egui::StrokeKind::Inside,
    );
    let at = rect.center() - galley.size() / 2.0;
    ui.painter().galley(at, galley, color);
}

pub fn progress(ui: &mut egui::Ui, fraction: f32, color: Color32, width: f32) {
    let (rect, _) = ui.allocate_exact_size(vec2(width, 6.0), egui::Sense::hover());
    ui.painter().rect_filled(rect, 3.0, ACCENT);
    let mut filled = rect;
    filled.set_width(rect.width() * fraction.clamp(0.0, 1.0));
    if filled.width() > 0.5 {
        ui.painter().rect_filled(filled, 3.0, color);
    }
}

pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(CARD)
        .stroke(Stroke::new(1.0, BORDER))
        .corner_radius(CARD_RADIUS)
        .inner_margin(Margin::same(14))
}

pub fn input<'a>(value: &'a mut String, hint: &str) -> egui::TextEdit<'a> {
    egui::TextEdit::singleline(value)
        .hint_text(RichText::new(hint).color(Color32::from_rgb(113, 113, 122)))
        .margin(Margin::symmetric(10, 8))
}

/// A label above its control.
pub fn field<R>(ui: &mut egui::Ui, label: &str, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 5.0;
        ui.label(RichText::new(label).font(medium(12.5)));
        add(ui)
    })
    .inner
}

pub fn switch(ui: &mut egui::Ui, on: &mut bool, label: &str) -> egui::Response {
    let galley = ui.painter().layout_no_wrap(label.to_string(), regular(13.5), FOREGROUND);
    let size = vec2(46.0 + galley.size().x, 24.0);
    let (rect, mut response) = ui.allocate_exact_size(size, egui::Sense::click());
    if response.clicked() {
        *on = !*on;
        response.mark_changed();
    }
    let (enabled, value) = (ui.is_enabled(), *on);
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, value, label));
    let t = ui.ctx().animate_bool_with_time(response.id, value, 0.15);
    let track = egui::Rect::from_min_size(egui::pos2(rect.left(), rect.center().y - 10.0), vec2(36.0, 20.0));
    ui.painter().rect_filled(track, 10.0, mix(INPUT, PRIMARY, t));
    let knob = egui::pos2(track.left() + 10.0 + 16.0 * t, track.center().y);
    ui.painter().circle_filled(knob, 8.0, mix(FOREGROUND, PRIMARY_FG, t));
    if response.has_focus() {
        ui.painter().rect_stroke(track.expand(2.0), 12.0, Stroke::new(2.0, RING), egui::StrokeKind::Outside);
    }
    let y = rect.center().y - galley.size().y / 2.0;
    ui.painter().galley_with_override_text_color(egui::pos2(rect.left() + 46.0, y), galley, FOREGROUND);
    response.on_hover_cursor(egui::CursorIcon::PointingHand)
}

/// A selectable chip, used for filters and weekdays.
pub fn chip(ui: &mut egui::Ui, selected: bool, label: &str) -> egui::Response {
    let kind = if selected { Kind::Primary } else { Kind::Outline };
    let response = button(ui, kind, "", label);
    let enabled = ui.is_enabled();
    response.widget_info(|| egui::WidgetInfo::selected(egui::Role::CheckBox, enabled, selected, label));
    response
}

/// A centred dialog over a dimmed window. Returns true when the user dismissed
/// it with Escape or the close button. Clicking outside does not dismiss it, so
/// a half-filled form is not lost by accident.
pub fn dialog(
    ctx: &egui::Context,
    id: &str,
    heading: &str,
    description: &str,
    width: f32,
    content: impl FnOnce(&mut egui::Ui),
) -> bool {
    let mut dismissed = false;
    let frame = card().inner_margin(Margin::same(22)).shadow(egui::Shadow {
        offset: [0, 18],
        blur: 48,
        spread: 0,
        color: Color32::from_black_alpha(160),
    });
    let response = egui::Modal::new(egui::Id::unique(id))
        .backdrop_color(Color32::from_black_alpha(175))
        .frame(frame)
        .show(ctx, |ui| {
            let screen = ui.ctx().content_rect();
            ui.set_width(width.min(screen.width() - 80.0));
            ui.horizontal(|ui| {
                ui.label(title(heading));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    dismissed = icon_button(ui, Kind::Ghost, icon::CLOSE, "Close").clicked();
                });
            });
            if !description.is_empty() {
                ui.add(egui::Label::new(muted(description)).wrap());
            }
            ui.add_space(6.0);
            // The body takes its natural height and scrolls only when the
            // window is too short for it.
            let height = screen.height() - 170.0;
            egui::ScrollArea::vertical()
                .max_height(height)
                .min_scrolled_height(height)
                .auto_shrink([false, true])
                .show(ui, content);
        });
    let escape = response.is_top_modal
        && !response.any_popup_open
        && ctx.input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Escape));
    dismissed || escape
}
