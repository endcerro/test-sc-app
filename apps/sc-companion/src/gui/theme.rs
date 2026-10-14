//! Dark, calm, instrument-panel look, ported from the hauling planner. Money, time and cargo
//! keep one colour each everywhere, so the pilot learns them.
//!
//! On Windows the UI font is Segoe UI with Segoe UI Symbol behind it, so symbols such as
//! `✕ ✓ ✗ ● ›` render; elsewhere egui's fonts are kept and [`sym`] falls back to plain text.

use eframe::egui::{self, Color32, CornerRadius, FontFamily, FontId, Id, Stroke, TextStyle};

pub const BG: Color32 = Color32::from_rgb(0x0d, 0x11, 0x17);
pub const PANEL: Color32 = Color32::from_rgb(0x15, 0x1b, 0x24);
pub const PANEL_2: Color32 = Color32::from_rgb(0x1b, 0x23, 0x30);
pub const LINE: Color32 = Color32::from_rgb(0x2a, 0x34, 0x43);
pub const TEXT: Color32 = Color32::from_rgb(0xdb, 0xe3, 0xee);
pub const MUTED: Color32 = Color32::from_rgb(0x85, 0x93, 0xa6);
/// Primary action, focus.
pub const ACCENT: Color32 = Color32::from_rgb(0x4f, 0xa3, 0xff);
/// Warnings, low-confidence values.
pub const WARN: Color32 = Color32::from_rgb(0xf0, 0xa3, 0x5b);
/// Problems that block planning.
pub const BAD: Color32 = Color32::from_rgb(0xef, 0x6b, 0x6b);
pub const GOOD: Color32 = Color32::from_rgb(0x7f, 0xd9, 0x9a);
pub const REWARD: Color32 = Color32::from_rgb(0xe8, 0xc1, 0x5a);
pub const CARGO: Color32 = Color32::from_rgb(0xa9, 0x9c, 0xf2);
pub const TIME: Color32 = Color32::from_rgb(0x5e, 0xc8, 0xe5);

pub const RADIUS: u8 = 8;

const SYMBOLS_ID: &str = "theme-symbol-font";

/// Segoe UI first, Segoe UI Symbol after egui's own fonts as a fallback. Both ship with
/// Windows; without them egui's fonts are kept. Returns whether the symbol font loaded.
fn fonts(ctx: &egui::Context) -> bool {
    let dir = std::path::Path::new("C:/Windows/Fonts");
    let mut defs = egui::FontDefinitions::default();
    let mut symbols = false;
    for (name, file, first) in [
        ("segoe-ui", "segoeui.ttf", true),
        ("segoe-symbol", "seguisym.ttf", false),
    ] {
        let Ok(bytes) = std::fs::read(dir.join(file)) else {
            continue;
        };
        defs.font_data.insert(
            name.into(),
            std::sync::Arc::new(egui::FontData::from_owned(bytes)),
        );
        let list = defs.families.entry(FontFamily::Proportional).or_default();
        if first {
            list.insert(0, name.into());
        } else {
            list.push(name.into());
            symbols = true;
        }
    }
    ctx.set_fonts(defs);
    symbols
}

/// `fancy` when the symbol font is loaded, else `plain`.
pub fn sym<'a>(ctx: &egui::Context, fancy: &'a str, plain: &'a str) -> &'a str {
    let loaded = ctx.data(|d| d.get_temp::<bool>(Id::new(SYMBOLS_ID)));
    if loaded.unwrap_or(false) {
        fancy
    } else {
        plain
    }
}

pub fn apply(ctx: &egui::Context) {
    let symbols = fonts(ctx);
    ctx.data_mut(|d| d.insert_temp(Id::new(SYMBOLS_ID), symbols));
    let mut style = (*ctx.style()).clone();
    style.text_styles = [
        (
            TextStyle::Heading,
            FontId::new(18.0, FontFamily::Proportional),
        ),
        (TextStyle::Body, FontId::new(14.0, FontFamily::Proportional)),
        (
            TextStyle::Button,
            FontId::new(14.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Small,
            FontId::new(12.0, FontFamily::Proportional),
        ),
        (
            TextStyle::Monospace,
            FontId::new(12.5, FontFamily::Monospace),
        ),
    ]
    .into();
    style.spacing.item_spacing = egui::vec2(8.0, 6.0);
    style.spacing.button_padding = egui::vec2(10.0, 5.0);
    style.spacing.interact_size.y = 28.0;

    let mut v = egui::Visuals::dark();
    v.panel_fill = BG;
    v.window_fill = PANEL;
    v.extreme_bg_color = BG;
    v.faint_bg_color = PANEL_2;
    v.override_text_color = Some(TEXT);
    v.hyperlink_color = ACCENT;
    v.warn_fg_color = WARN;
    v.error_fg_color = BAD;
    v.selection.bg_fill = ACCENT.linear_multiply(0.45);
    v.selection.stroke = Stroke::new(1.0_f32, ACCENT);
    v.window_stroke = Stroke::new(1.0_f32, LINE);
    let r = CornerRadius::same(4);
    for w in [
        &mut v.widgets.noninteractive,
        &mut v.widgets.inactive,
        &mut v.widgets.hovered,
        &mut v.widgets.active,
        &mut v.widgets.open,
    ] {
        w.corner_radius = r;
    }
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0_f32, TEXT);
    v.widgets.inactive.bg_fill = PANEL_2;
    v.widgets.inactive.weak_bg_fill = PANEL_2;
    v.widgets.inactive.bg_stroke = Stroke::new(1.0_f32, LINE);
    v.widgets.hovered.bg_fill = PANEL_2;
    v.widgets.hovered.weak_bg_fill = PANEL_2;
    v.widgets.hovered.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    v.widgets.active.bg_stroke = Stroke::new(1.0_f32, ACCENT);
    style.visuals = v;
    ctx.set_style(style);
}

/// A card: panel background, thin border, rounded.
pub fn card() -> egui::Frame {
    egui::Frame::new()
        .fill(PANEL)
        .stroke(Stroke::new(1.0_f32, LINE))
        .corner_radius(RADIUS)
        .inner_margin(10)
}
