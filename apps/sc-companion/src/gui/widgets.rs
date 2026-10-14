//! Small widgets shared by the Contracts and Routes panes, ported from the hauling planner.
//!
//! Text fields edit a copy and commit on Enter or when focus leaves: half-typed numbers
//! never reach the contract, and pressing Enter on an unchanged value still commits it
//! (which confirms a flagged read).

use eframe::egui::{
    self, Color32, Id, Key, Pos2, Rect, Response, RichText, Sense, Shape, Stroke, StrokeKind,
    TextEdit, Ui,
};

use crate::contracts::places::PlaceTree;
use crate::contracts::review::Mark;
use crate::gui::theme;

pub struct FieldOut {
    pub response: Response,
    /// The new text, when the player committed one.
    pub commit: Option<String>,
}

fn paint_mark(ui: &Ui, rect: Rect, mark: Mark) {
    let color = match mark {
        Mark::None => return,
        Mark::Unsure => theme::WARN,
        Mark::Bad => theme::BAD,
    };
    let p = ui.painter();
    p.rect_filled(
        Rect::from_min_size(rect.min, egui::vec2(3.0, rect.height())),
        2.0,
        color,
    );
    if mark == Mark::Bad {
        p.rect_stroke(rect, 6.0, Stroke::new(1.0_f32, color), StrokeKind::Inside);
    } else {
        let r = rect.shrink(0.5);
        let pts = [
            r.left_top(),
            r.right_top(),
            r.right_bottom(),
            r.left_bottom(),
            r.left_top(),
        ];
        p.extend(Shape::dashed_line(
            &pts,
            Stroke::new(1.0_f32, color),
            4.0,
            3.0,
        ));
    }
}

/// Inner margin of a text field, which egui adds around its `desired_width`.
const FIELD_MARGIN: egui::Margin = egui::Margin::symmetric(4, 2);

/// One-line text field with an edit buffer, `width` wide in all (margin included).
/// `color` tints the text (reward, cargo...).
pub fn field(
    ui: &mut Ui,
    id: Id,
    value: &str,
    width: f32,
    mark: Mark,
    color: Option<Color32>,
) -> FieldOut {
    let editing = ui.memory(|m| m.has_focus(id));
    let mut buf = if editing {
        ui.data(|d| d.get_temp::<String>(id))
            .unwrap_or_else(|| value.to_owned())
    } else {
        value.to_owned()
    };
    let inner = (width - FIELD_MARGIN.sum().x).max(8.0);
    let response = ui.add(
        TextEdit::singleline(&mut buf)
            .id(id)
            .margin(FIELD_MARGIN)
            .desired_width(inner)
            .text_color_opt(color),
    );
    paint_mark(ui, response.rect, mark);
    let mut commit = None;
    if response.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, buf.clone()));
    } else if response.lost_focus() {
        ui.data_mut(|d| d.remove::<String>(id));
        let enter = ui.input(|i| i.key_pressed(Key::Enter));
        if buf != value || enter {
            commit = Some(buf);
        }
    }
    FieldOut { response, commit }
}

/// A field for numbers; commits only what parses (`parse` returns None to reject).
pub fn number<T>(
    ui: &mut Ui,
    id: Id,
    value: &str,
    width: f32,
    parse: impl Fn(&str) -> Option<T>,
) -> Option<T> {
    field(ui, id, value, width, Mark::None, None)
        .commit
        .and_then(|t| parse(&t))
}

/// The line under a place field saying where the name resolved: `✓ Stanton › Crusader
/// › Yela`, the candidates of a shared name, or that it's unknown. Truncated to `width`;
/// the whole of it on hover.
pub fn place_tree(ui: &mut Ui, tree: &PlaceTree, width: f32) {
    let ctx = ui.ctx().clone();
    let (text, color, tip) = match tree {
        PlaceTree::Empty => return,
        PlaceTree::Known { path, no_distance } => {
            let mut text = format!(
                "{} {}",
                theme::sym(&ctx, "\u{2713}", "ok:"),
                path.join(" \u{203a} ")
            );
            if *no_distance {
                text.push_str(" \u{b7} no distance");
            }
            let color = if *no_distance {
                theme::WARN
            } else {
                theme::MUTED
            };
            (text, color, None)
        }
        PlaceTree::Ambiguous(paths) => {
            let last: Vec<&str> = paths
                .iter()
                .map(|p| p.last().map_or("?", String::as_str))
                .collect();
            let full = paths
                .iter()
                .map(|p| p.join(" \u{203a} "))
                .collect::<Vec<_>>()
                .join("\n");
            (
                format!("? {} places: {}", paths.len(), last.join(" \u{b7} ")),
                theme::WARN,
                Some(format!("Several places have this name:\n{full}")),
            )
        }
        PlaceTree::Unknown => (
            format!("{} unknown place", theme::sym(&ctx, "\u{2717}", "x")),
            theme::BAD,
            None,
        ),
    };
    let tip = tip.unwrap_or_else(|| text.clone());
    ui.allocate_ui(egui::vec2(width, 14.0), |ui| {
        ui.set_width(width);
        ui.add(egui::Label::new(RichText::new(text).size(11.0).color(color)).truncate())
            .on_hover_text(tip);
    });
}

/// How many places a place field lists.
pub const PLACE_CHOICES: usize = 8;

/// Tallest the list of places under a place field gets (it scrolls past that).
const PICK_MAX_H: f32 = 220.0;

/// Offers places for what's typed: (name, system), best first.
pub type Suggest<'a> = &'a dyn Fn(&str) -> Vec<(String, String)>;

/// A location field: as the player types, known places are offered underneath, and on
/// focus an unknown name already lists what it may mean. Click a choice, or pick it with
/// Down/Up and Enter (Enter with nothing picked keeps the text as typed).
pub fn place(
    ui: &mut Ui,
    id: Id,
    value: &str,
    width: f32,
    mark: Mark,
    suggest: Suggest,
) -> FieldOut {
    let pick_id = id.with("pick");
    let hl_id = id.with("highlight");
    // Read before `field`, which drops the edit buffer when focus goes.
    let typed = ui.data(|d| d.get_temp::<String>(id));
    let highlight = ui.data(|d| d.get_temp::<usize>(hl_id));
    let mut out = field(ui, id, value, width, mark, None);
    let query = typed.unwrap_or_else(|| value.to_owned());

    if out.response.lost_focus()
        && ui.input(|i| i.key_pressed(Key::Enter))
        && let Some(name) = highlight.and_then(|h| suggest(&query).into_iter().nth(h))
    {
        out.commit = Some(name.0);
    }
    // The list stays up while the mouse is on it, since pressing a choice takes focus away.
    let over_list = ui.data(|d| d.get_temp::<bool>(pick_id)).unwrap_or(false);
    let clear = |ui: &mut Ui| {
        ui.data_mut(|d| {
            d.remove::<bool>(pick_id);
            d.remove::<usize>(hl_id);
        });
    };
    if !(out.response.has_focus() || over_list) {
        clear(ui);
        return out;
    }
    let matches = suggest(&query);
    if matches.is_empty() || (matches.len() == 1 && matches[0].0.eq_ignore_ascii_case(query.trim()))
    {
        clear(ui);
        return out;
    }
    let mut highlight = highlight.filter(|&h| h < matches.len());
    let mut moved = false;
    if out.response.has_focus() {
        let (down, up) = ui.input(|i| (i.key_pressed(Key::ArrowDown), i.key_pressed(Key::ArrowUp)));
        if down {
            highlight = Some(highlight.map_or(0, |h| (h + 1).min(matches.len() - 1)));
        } else if up {
            highlight = highlight.and_then(|h| h.checked_sub(1));
        }
        moved = down || up;
    }
    // At most PICK_MAX_H tall, and never past the bottom of the window: the rest scrolls.
    let top = out.response.rect.bottom() + 2.0;
    let room = ui.ctx().content_rect().bottom() - top - 12.0;
    let max_h = PICK_MAX_H.min(room).max(60.0);
    let area = egui::Area::new(pick_id)
        .order(egui::Order::Foreground)
        .fixed_pos(out.response.rect.left_bottom() + egui::vec2(0.0, 2.0))
        .show(ui.ctx(), |ui| {
            egui::Frame::popup(ui.style()).show(ui, |ui| {
                ui.set_min_width(width.max(220.0));
                egui::ScrollArea::vertical()
                    .id_salt(pick_id.with("scroll"))
                    .max_height(max_h)
                    .show(ui, |ui| {
                        for (n, (name, system)) in matches.iter().enumerate() {
                            let on = highlight == Some(n);
                            let button = egui::Button::new(
                                RichText::new(format!("{name}  ")).color(theme::TEXT),
                            )
                            .frame(on)
                            .selected(on)
                            .right_text(RichText::new(system).color(theme::MUTED).small());
                            let r = ui.add(button);
                            if on && moved {
                                r.scroll_to_me(None);
                            }
                            if r.clicked() {
                                out.commit = Some(name.clone());
                            }
                        }
                    });
            })
        });
    let hovered = area.response.contains_pointer();
    ui.data_mut(|d| {
        if hovered && out.commit.is_none() {
            d.insert_temp(pick_id, true);
        } else {
            d.remove::<bool>(pick_id);
        }
        if let Some(h) = highlight {
            d.insert_temp(hl_id, h);
        } else {
            d.remove::<usize>(hl_id);
        }
    });
    out
}

/// A small uppercase label above a widget.
pub fn labeled<R>(
    ui: &mut Ui,
    label: &str,
    add: impl FnOnce(&mut Ui) -> R,
) -> egui::InnerResponse<R> {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 3.0;
        ui.label(
            RichText::new(label.to_uppercase())
                .size(11.0)
                .color(theme::MUTED),
        );
        add(ui)
    })
}

pub fn small_button(ui: &mut Ui, text: &str) -> Response {
    ui.add(egui::Button::new(RichText::new(text).size(13.0)))
}

pub fn primary_button(ui: &mut Ui, text: &str, enabled: bool) -> Response {
    ui.add_enabled(
        enabled,
        egui::Button::new(
            RichText::new(text)
                .strong()
                .color(Color32::from_rgb(0x06, 0x12, 0x1f)),
        )
        .fill(theme::ACCENT),
    )
}

/// A frameless ✕ that turns red on hover.
pub fn remove_button(ui: &mut Ui, tip: &str) -> bool {
    let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), Sense::click());
    let color = if r.hovered() {
        theme::BAD
    } else {
        theme::MUTED
    };
    let glyph = theme::sym(ui.ctx(), "\u{2715}", "x");
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        glyph,
        egui::FontId::proportional(13.0),
        color,
    );
    r.on_hover_text(tip).clicked()
}

/// A frameless +, greyed out (with `disabled_tip`) when it can't be used.
pub fn add_button(ui: &mut Ui, tip: &str, enabled: bool, disabled_tip: &str) -> bool {
    let sense = if enabled {
        Sense::click()
    } else {
        Sense::hover()
    };
    let (rect, r) = ui.allocate_exact_size(egui::vec2(22.0, 22.0), sense);
    let color = match (enabled, r.hovered()) {
        (false, _) => theme::LINE,
        (true, true) => theme::ACCENT,
        (true, false) => theme::MUTED,
    };
    ui.painter().text(
        rect.center(),
        egui::Align2::CENTER_CENTER,
        "+",
        egui::FontId::proportional(15.0),
        color,
    );
    r.on_hover_text(if enabled { tip } else { disabled_tip })
        .clicked()
}

/// A small rounded badge (the fit indicator).
pub fn badge(ui: &mut Ui, text: &str, color: Color32, border: Color32) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0_f32, border))
        .corner_radius(99)
        .inner_margin(egui::Margin::symmetric(8, 2))
        .show(ui, |ui| {
            ui.label(RichText::new(text).size(12.0).color(color))
        });
}

pub fn dashed_segment(points: [Pos2; 2], stroke: Stroke, dash: f32, gap: f32) -> Vec<Shape> {
    Shape::dashed_line(&points, stroke, dash, gap)
}
