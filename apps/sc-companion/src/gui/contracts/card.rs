//! One contract on the board. Every card has the compact header line (fold/unfold, status,
//! Edit/Done). Finalized contracts show the compact read-only summary
//! ([`contract_view::body`]); the others the editor, ported from the hauling planner's
//! review card: problems and fixes, the doubtful reads to check, header fields, one row per
//! cargo line (SCU | cargo | pickups | drop-offs | loaded), each place with where it
//! resolved, and the capture: the pixels the focused field was read from (also on hover), or
//! the whole screenshots. Right-clicking a doubtful (amber) field marks it checked.

use std::collections::HashMap;

use eframe::egui::{self, Align, Id, Rect, RichText, Stroke, StrokeKind, Ui};
use sc_universe::Universe;

use super::{Edit, Reviewed, Textures, View, format};
use crate::contracts::Entry;
use crate::contracts::fields::{self, Field};
use crate::contracts::places::{PlaceTree, Places};
use crate::contracts::review::{Mark, Status};
use crate::contracts::sources::{self, Source};
use crate::gui::contract_view::{self, Header};
use crate::gui::theme;
use crate::gui::widgets::{
    FieldOut, PLACE_CHOICES, add_button, field, labeled, place, place_tree, primary_button,
    remove_button, small_button,
};

/// Width of a 90-character title, the longest the game shows.
const TITLE_MAX_W: f32 = 640.0;

/// What a card needs while drawing its fields.
pub struct Cx<'a> {
    pub entry: &'a Entry,
    /// `#N` on the board.
    pub number: usize,
    pub reviewed: &'a Reviewed,
    pub universe: &'a Universe,
    pub places: &'a Places,
    pub trees: &'a mut HashMap<String, PlaceTree>,
    pub view: &'a mut View,
    pub edits: &'a mut Vec<Edit>,
    /// Set to the field with keyboard focus.
    pub focused: &'a mut Option<(u64, Field)>,
    /// Board numbers of the contracts this one looks like.
    pub duplicates: Vec<usize>,
    /// Folded down to its header line.
    pub collapsed: bool,
}

impl Cx<'_> {
    fn id(&self) -> u64 {
        self.entry.id
    }

    fn field(
        &mut self,
        ui: &mut Ui,
        f: Field,
        width: f32,
        color: Option<egui::Color32>,
    ) -> FieldOut {
        let id = Id::new(("contract-field", self.id(), f));
        let text = fields::text(&self.entry.contract, f);
        let mark = self.reviewed.marks.get(&f).copied().unwrap_or(Mark::None);
        let (u, places) = (self.universe, self.places);
        let mut out = if f.is_place() {
            place(ui, id, &text, width, mark, &|q| {
                places.suggest(u, q, PLACE_CHOICES)
            })
        } else {
            field(ui, id, &text, width, mark, color)
        };
        if self.view.goto == Some((self.id(), f)) {
            out.response.request_focus();
            out.response.scroll_to_me(Some(Align::Center));
            self.view.goto = None;
        }
        if out.response.has_focus() {
            *self.focused = Some((self.id(), f));
        }
        // Doubtful reads (and cargo names snapping changed) are checked with a right-click.
        let checkable = self.reviewed.unsure.contains(&f);
        if checkable && out.response.secondary_clicked() {
            self.edits.push(Edit::Confirm(self.id(), vec![f]));
        }
        let hint = match mark {
            Mark::Bad => Some(("Missing or unknown: needed to plan", theme::BAD)),
            _ if checkable => Some(("Right-click when it reads right", theme::WARN)),
            Mark::Unsure | Mark::None => None,
        };
        let src = sources::locate(self.entry, f);
        if out.response.hovered() && (src.is_some() || hint.is_some()) {
            let tex = src.and_then(|s| self.view.textures.of_shot(ui.ctx(), self.entry, s.shot));
            let max_w = 520.0f32.min(ui.ctx().content_rect().width() * 0.5);
            out.response = out.response.on_hover_ui(|ui| {
                if let Some((text, color)) = hint {
                    ui.label(RichText::new(text).color(color).size(12.5));
                }
                let Some(src) = src else {
                    return;
                };
                ui.label(
                    RichText::new(format!("Read as the {}:", f.label()))
                        .color(theme::MUTED)
                        .size(12.0),
                );
                match &tex {
                    Some(t) => crop(ui, t, src, 56.0, max_w),
                    None => {
                        ui.label("(screenshot missing)");
                    }
                }
            });
        }
        if let Some(v) = out.commit.clone() {
            self.edits.push(Edit::Set(self.id(), f, v));
        }
        out
    }

    fn tree(&mut self, name: &str) -> PlaceTree {
        let (u, places) = (self.universe, self.places);
        self.trees
            .entry(name.trim().to_owned())
            .or_insert_with(|| places.tree(u, name))
            .clone()
    }
}

/// Field `j` of line `k` of one place list.
type FieldOf = fn(usize, usize) -> Field;

/// Pickups or drop-offs of a line.
#[derive(Clone, Copy, PartialEq)]
enum Leg {
    Pickup,
    Dropoff,
}

/// The pickups (or drop-offs) of line `k`, one field each, each with the place's path under
/// it; drop-offs of a line with several also get their own SCU. "+" opens an empty slot for
/// one more (only while the other side has a single place: a line has one pickup or one
/// drop-off); ✕ removes one (when there are several) or closes the slot.
fn places(ui: &mut Ui, cx: &mut Cx, k: usize, width: f32, leg: Leg) {
    let (id, entry) = (cx.id(), cx.entry);
    let Some(flow) = entry.contract.flows.get(k) else {
        return;
    };
    let (count, other, field, noun): (usize, usize, FieldOf, &str) = match leg {
        Leg::Pickup => (
            flow.pickups.len(),
            flow.dropoffs.len(),
            Field::Pickup,
            "pickup",
        ),
        Leg::Dropoff => (
            flow.dropoffs.len(),
            flow.pickups.len(),
            Field::Dropoff,
            "drop-off",
        ),
    };
    let several_drops = leg == Leg::Dropoff && count > 1;
    let can_add = other <= 1;
    let slots = match leg {
        Leg::Pickup => &cx.view.new_pickup,
        Leg::Dropoff => &cx.view.new_dropoff,
    };
    let slot = slots.contains(&(id, k));
    // A line without any still shows one (empty) field to type into.
    let shown = count.max(1) + usize::from(slot && count > 0);
    let scu_w = if several_drops { 40.0 } else { 0.0 };
    let field_w = (width - 2.0 * 24.0 - scu_w).max(24.0);
    ui.vertical(|ui| {
        ui.set_width(width);
        for j in 0..shown {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let out = cx.field(ui, field(k, j), field_w, None);
                if several_drops && j < count {
                    ui.add_space(2.0);
                    cx.field(ui, Field::DropScu(k, j), scu_w - 4.0, Some(theme::CARGO))
                        .response
                        .on_hover_text("SCU to this drop-off");
                }
                let slots = match leg {
                    Leg::Pickup => &mut cx.view.new_pickup,
                    Leg::Dropoff => &mut cx.view.new_dropoff,
                };
                let is_slot = j >= count && count > 0;
                if is_slot {
                    if out.commit.is_some() {
                        slots.remove(&(id, k));
                    }
                    if remove_button(ui, &format!("Close this empty {noun}")) {
                        slots.remove(&(id, k));
                    }
                } else if count > 1 && remove_button(ui, &format!("Remove this {noun}")) {
                    cx.edits.push(Edit::Set(id, field(k, j), String::new()));
                } else if count <= 1 {
                    ui.add_space(22.0);
                }
                if j + 1 == shown
                    && !slot
                    && count > 0
                    && add_button(
                        ui,
                        &format!("Add a {noun}"),
                        can_add,
                        "A line has one pickup or one drop-off: remove the extra places on \
                         the other side first",
                    )
                {
                    slots.insert((id, k));
                }
            });
            let name = fields::text(&entry.contract, field(k, j));
            let tree = cx.tree(&name);
            place_tree(ui, &tree, width);
        }
    });
}

pub fn show(ui: &mut Ui, cx: &mut Cx) {
    let (id, entry, reviewed) = (cx.id(), cx.entry, cx.reviewed);
    let status = reviewed.status;
    let frame = theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        let h = Header {
            number: cx.number,
            contract: &entry.contract,
            status: Some(status),
            collapsed: cx.collapsed,
        };
        if contract_view::header(ui, &h, |ui| header_buttons(ui, cx)) {
            cx.edits.push(Edit::ToggleCollapsed(id));
        }
        if cx.view.confirm_remove == Some(id) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("Remove this contract?").color(theme::BAD));
                if small_button(ui, "Remove").clicked() {
                    cx.edits.push(Edit::RemoveContract(id));
                }
                if small_button(ui, "Keep").clicked() {
                    cx.view.confirm_remove = None;
                }
            });
        }
        if cx.collapsed {
            return;
        }
        ui.add_space(2.0);
        if entry.done {
            finalized(ui, cx);
        } else {
            editor(ui, cx);
        }
    });
    let color = match status {
        Status::Ready => return,
        Status::ToCheck(_) => theme::WARN,
        Status::Blocked(_) => theme::BAD,
    };
    let r = frame.response.rect;
    ui.painter().rect_filled(
        Rect::from_min_size(r.min, egui::vec2(4.0, r.height())),
        2.0,
        color,
    );
}

/// Right end of the header: Done (editing) or Edit (finalized), and the "..." menu.
fn header_buttons(ui: &mut Ui, cx: &mut Cx) {
    let (id, entry) = (cx.id(), cx.entry);
    let dots = theme::sym(ui.ctx(), "\u{22ef}", "...");
    ui.menu_button(RichText::new(dots).size(14.0), |ui| {
        if !entry.shots.is_empty() {
            let open = cx.view.open_shots.contains(&id);
            let text = if open {
                "Hide screenshots"
            } else {
                "Show screenshots"
            };
            if ui.button(text).clicked() {
                cx.edits.push(Edit::ToggleShot(id));
                if cx.collapsed {
                    cx.edits.push(Edit::ToggleCollapsed(id));
                }
            }
            let reparse = if entry.edited {
                "Re-parse from screenshots (discards edits)"
            } else {
                "Re-parse from screenshots"
            };
            if ui.button(reparse).clicked() {
                cx.edits.push(Edit::Reparse(id));
            }
        }
        if ui
            .button(RichText::new("Remove contract").color(theme::BAD))
            .clicked()
        {
            cx.view.confirm_remove = Some(id);
        }
    })
    .response
    .on_hover_text("More");
    if entry.done {
        if small_button(ui, "Edit")
            .on_hover_text("Open the editor to change or check values")
            .clicked()
        {
            cx.edits.push(Edit::Reopen(id));
        }
    } else if primary_button(ui, "Done", true)
        .on_hover_text("Finalize: the values look right (accepts the amber reads), show it compact")
        .clicked()
    {
        cx.edits.push(Edit::Finish(id));
    }
}

/// A finalized contract: the compact summary, what still blocks planning, and the footer.
fn finalized(ui: &mut Ui, cx: &mut Cx) {
    let (id, entry) = (cx.id(), cx.entry);
    let ticked = contract_view::body(ui, &entry.contract, &mut |n| cx.tree(n), true);
    if let Some((k, on)) = ticked {
        cx.edits.push(Edit::Loaded(id, k, on));
    }
    problems(ui, cx);
    notes(ui, cx);
    footer(ui, cx);
}

/// The editor: problems, the reads to check, every field, the capture.
fn editor(ui: &mut Ui, cx: &mut Cx) {
    let (id, entry) = (cx.id(), cx.entry);
    let c = &entry.contract;
    problems(ui, cx);
    to_check(ui, cx);
    labeled(ui, "Title", |ui| {
        // Titles run to 90 characters at most.
        let w = ui.available_width().min(TITLE_MAX_W);
        cx.field(ui, Field::Title, w, None)
    });
    ui.horizontal_wrapped(|ui| {
        labeled(ui, "Reward (aUEC)", |ui| {
            cx.field(ui, Field::Reward, 110.0, Some(theme::REWARD))
        });
        labeled(ui, "Contractor", |ui| {
            cx.field(ui, Field::Contractor, 170.0, None)
        });
        labeled(ui, "On board for", |ui| {
            cx.field(ui, Field::Availability, 80.0, None)
        })
        .response
        .on_hover_text("How long the offer stays on the board");
        labeled(ui, "Largest box", |ui| {
            cx.field(ui, Field::MaxBox, 60.0, None)
        });
        labeled(ui, "Rep", |ui| cx.field(ui, Field::Rep, 50.0, None));
    });
    ui.add_space(4.0);

    // Column widths come from the card's width, never more.
    let gap = 6.0;
    let (check_w, x_w) = (46.0, 22.0);
    let room = (ui.available_width() - check_w - x_w - 5.0 * gap).max(0.0);
    let scu_w = (room * 0.12).clamp(28.0, 52.0);
    let cargo_w = (room * 0.26).clamp(60.0, 160.0);
    let place_w = ((room - scu_w - cargo_w) / 2.0).max(0.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = gap;
        for (t, cw) in [
            ("SCU", scu_w),
            ("Cargo", cargo_w),
            ("Pickups", place_w),
            ("Drop-offs", place_w),
            ("Loaded", check_w),
        ] {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(cw, 14.0), egui::Sense::hover());
            ui.painter().text(
                rect.left_center(),
                egui::Align2::LEFT_CENTER,
                t.to_uppercase(),
                egui::FontId::proportional(11.0),
                theme::MUTED,
            );
        }
    });
    for (k, flow) in c.flows.iter().enumerate() {
        if k > 0 {
            ui.add(egui::Separator::default().spacing(2.0));
        }
        ui.with_layout(egui::Layout::left_to_right(Align::TOP), |ui| {
            ui.spacing_mut().item_spacing.x = gap;
            if flow.dropoffs.len() > 1 {
                let (rect, r) =
                    ui.allocate_exact_size(egui::vec2(scu_w, 24.0), egui::Sense::hover());
                ui.painter().text(
                    rect.left_center() + egui::vec2(5.0, 0.0),
                    egui::Align2::LEFT_CENTER,
                    flow.scu().to_string(),
                    egui::FontId::proportional(14.0),
                    theme::CARGO,
                );
                r.on_hover_text("Total of the drop-offs");
            } else {
                cx.field(ui, Field::Scu(k), scu_w, Some(theme::CARGO));
            }
            cx.field(ui, Field::Commodity(k), cargo_w, None)
                .response
                .on_hover_text("Cargo type; start with [!] for restricted cargo");
            places(ui, cx, k, place_w, Leg::Pickup);
            places(ui, cx, k, place_w, Leg::Dropoff);
            let mut on = flow.loaded;
            ui.add_space(14.0);
            if ui
                .checkbox(&mut on, "")
                .on_hover_text("Already on board")
                .changed()
            {
                cx.edits.push(Edit::Loaded(id, k, on));
            }
            ui.add_space(check_w - 14.0 - 24.0);
            if remove_button(ui, "Remove this cargo line") {
                cx.edits.push(Edit::RemoveLine(id, k));
            }
        });
    }
    ui.horizontal(|ui| {
        if small_button(ui, "Add cargo line").clicked() {
            cx.edits.push(Edit::AddLine(id));
        }
        if entry.stale && small_button(ui, "Re-parse (discards edits)").clicked() {
            cx.edits.push(Edit::Reparse(id));
        }
    });

    if !entry.shots.is_empty() {
        capture_strip(ui, cx);
    }
    notes(ui, cx);
    footer(ui, cx);
}

/// What blocks planning, in a red box, with the "Use X for Y" fixes.
fn problems(ui: &mut Ui, cx: &mut Cx) {
    let (id, review) = (cx.id(), &cx.reviewed.review);
    if review.problems.is_empty() && review.fixes.is_empty() {
        return;
    }
    egui::Frame::new()
        .fill(theme::BAD.gamma_multiply(0.08))
        .stroke(Stroke::new(1.0_f32, theme::BAD.gamma_multiply(0.6)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            for p in &review.problems {
                ui.label(RichText::new(p).color(theme::BAD).size(13.0));
            }
            if !review.fixes.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    for fix in &review.fixes {
                        let label = format!(
                            "Use \u{201c}{}\u{201d} for \u{201c}{}\u{201d}",
                            fix.suggestion, fix.written
                        );
                        if small_button(ui, &label).clicked() {
                            cx.edits.push(Edit::Fix(
                                id,
                                fix.written.clone(),
                                fix.suggestion.clone(),
                            ));
                        }
                    }
                });
            }
        });
}

/// The doubtful reads as chips: click one to go to its field, its tick to accept it; or
/// accept them all.
fn to_check(ui: &mut Ui, cx: &mut Cx) {
    let (id, entry) = (cx.id(), cx.entry);
    let unsure = &cx.reviewed.unsure;
    if unsure.is_empty() {
        return;
    }
    let tick = theme::sym(ui.ctx(), "\u{2713}", "ok");
    let round = |left: bool| {
        let (l, r) = if left { (10, 0) } else { (0, 10) };
        egui::CornerRadius {
            nw: l,
            sw: l,
            ne: r,
            se: r,
        }
    };
    egui::Frame::new()
        .fill(theme::WARN.gamma_multiply(0.07))
        .stroke(Stroke::new(1.0_f32, theme::WARN.gamma_multiply(0.5)))
        .corner_radius(6)
        .inner_margin(egui::Margin::symmetric(8, 6))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new("TO CHECK")
                        .size(11.0)
                        .color(theme::WARN)
                        .strong(),
                )
                .on_hover_text("Doubtful reads. Right-click a field when it reads right.");
                let pad = 2.0 * ui.spacing().button_padding.x;
                let groups = check_groups(entry, unsure);
                let several = groups.len() > 1;
                for (what, group) in groups {
                    let first = group[0];
                    let text = if group.len() > 1 {
                        format!("{what} \u{d7}{}", group.len())
                    } else {
                        what
                    };
                    let go = RichText::new(text).size(12.5).color(theme::WARN);
                    let ok = RichText::new(tick).size(12.5).color(theme::GOOD);
                    // Keep a chip and its tick together: wrap before the pair, never inside.
                    let width = |t: &RichText| {
                        egui::WidgetText::from(t.clone())
                            .into_galley(
                                ui,
                                Some(egui::TextWrapMode::Extend),
                                f32::INFINITY,
                                egui::TextStyle::Button,
                            )
                            .size()
                            .x
                            + pad
                    };
                    let need = width(&go) + width(&ok) + 1.0;
                    if need > ui.available_width() {
                        ui.end_row();
                    }
                    let go = egui::Button::new(go)
                        .wrap_mode(egui::TextWrapMode::Truncate)
                        .corner_radius(round(true));
                    if ui.add(go).on_hover_text("Go to this field").clicked() {
                        cx.view.goto = Some((id, first));
                    }
                    ui.add_space(1.0 - ui.spacing().item_spacing.x);
                    let ok = egui::Button::new(ok).corner_radius(round(false));
                    let tip = if group.len() > 1 {
                        "They read right (or right-click a field)"
                    } else {
                        "It reads right (or right-click the field)"
                    };
                    if ui.add(ok).on_hover_text(tip).clicked() {
                        cx.edits.push(Edit::Confirm(id, group));
                    }
                }
                if several
                    && ui
                        .add(egui::Button::new(RichText::new("Accept all").size(12.5)).frame(false))
                        .on_hover_text("Every amber value reads right")
                        .clicked()
                {
                    cx.edits.push(Edit::Confirm(id, unsure.clone()));
                }
            });
        });
}

/// The doubtful reads by what the chip says (`Baijini Point drop-off`), in reading order:
/// one chip for the same value on several lines, since confirming one confirms them all.
fn check_groups(entry: &Entry, unsure: &[Field]) -> Vec<(String, Vec<Field>)> {
    let mut groups: Vec<(String, Vec<Field>)> = Vec::new();
    for &f in unsure {
        let what = match f {
            _ if f.is_place() => format!("{} {}", fields::text(&entry.contract, f), f.label()),
            Field::Commodity(_) => format!("{} cargo", fields::text(&entry.contract, f)),
            _ => f.label().to_owned(),
        };
        match groups.iter_mut().find(|g| g.0 == what) {
            Some(g) => g.1.push(f),
            None => groups.push((what, vec![f])),
        }
    }
    groups
}

/// Amber notes that are not a field to tick (stale merge, rounded reward), and duplicates.
fn notes(ui: &mut Ui, cx: &mut Cx) {
    // Cargo types snapping changed are among the reads to check.
    for n in &cx.reviewed.review.notes {
        ui.label(
            RichText::new(format!("Check: {n}"))
                .color(theme::WARN)
                .size(12.5),
        );
    }
    if !cx.duplicates.is_empty() {
        let list: Vec<String> = cx.duplicates.iter().map(|n| format!("#{n}")).collect();
        ui.label(
            RichText::new(format!("Looks like a duplicate of {}", list.join(", ")))
                .color(theme::WARN)
                .size(12.5),
        );
    }
}

fn footer(ui: &mut Ui, cx: &mut Cx) {
    ui.label(
        RichText::new(format::footer(cx.entry))
            .color(theme::MUTED)
            .size(12.0),
    );
}

/// Under the cargo lines of a captured contract: the source of the focused field, or a
/// button to see the whole screenshots as captured.
fn capture_strip(ui: &mut Ui, cx: &mut Cx) {
    let (id, entry) = (cx.id(), cx.entry);
    let open = cx.view.open_shots.contains(&id);
    let focus = cx.view.focus.filter(|(fid, _)| *fid == id).map(|(_, f)| f);
    ui.horizontal(|ui| {
        let text = if open { "Hide capture" } else { "Show capture" };
        if ui
            .add(egui::Button::new(RichText::new(text).size(12.5)).frame(false))
            .clicked()
        {
            cx.edits.push(Edit::ToggleShot(id));
        }
        if let Some(f) = focus {
            ui.label(
                RichText::new(format!("\u{b7} read as the {}:", f.label()))
                    .color(theme::MUTED)
                    .size(12.5),
            );
        }
    });
    if let Some(src) = focus.and_then(|f| sources::locate(entry, f))
        && let Some(t) = cx.view.textures.of_shot(ui.ctx(), entry, src.shot)
    {
        crop(ui, &t, src, 64.0, ui.available_width());
    }
    if open {
        for shot in 0..entry.shots.len() {
            if let Some(t) = cx.view.textures.of_shot(ui.ctx(), entry, shot) {
                let size = t.size_vec2();
                let w = ui.available_width().min(size.x);
                ui.add(
                    egui::Image::from_texture(&t)
                        .fit_to_exact_size(egui::vec2(w, size.y * w / size.x)),
                );
            } else {
                ui.label(RichText::new("(screenshot moved or deleted)").color(theme::MUTED));
            }
        }
    }
}

/// Shows the part of a screenshot around `b`, `height` pixels tall (at most `max_w` wide),
/// with the value's box outlined.
fn crop(ui: &mut Ui, tex: &egui::TextureHandle, b: Source, height: f32, max_w: f32) {
    let size = tex.size_vec2();
    let (x0, y0) = (b.x as f32, b.y as f32);
    let (x1, y1) = (x0 + b.w as f32, y0 + b.h as f32);
    let region = Rect::from_min_max(
        egui::pos2(x0 - 24.0, y0 - 8.0),
        egui::pos2(x1 + 24.0, y1 + 8.0),
    )
    .intersect(Rect::from_min_size(egui::Pos2::ZERO, size));
    if region.width() < 1.0 || region.height() < 1.0 {
        return;
    }
    let scale = (height / region.height()).min(max_w / region.width());
    let shown = region.size() * scale;
    let uv = Rect::from_min_max(
        (region.min.to_vec2() / size).to_pos2(),
        (region.max.to_vec2() / size).to_pos2(),
    );
    let r = ui.add(
        egui::Image::from_texture(tex)
            .uv(uv)
            .fit_to_exact_size(shown)
            .maintain_aspect_ratio(false),
    );
    let to_screen = |x: f32, y: f32| r.rect.min + (egui::pos2(x, y) - region.min) * scale;
    let mark = Rect::from_min_max(to_screen(x0, y0), to_screen(x1, y1)).expand(2.0);
    ui.painter().rect_stroke(
        mark,
        3.0,
        Stroke::new(1.5_f32, theme::ACCENT),
        StrokeKind::Outside,
    );
}

impl Textures {
    /// The screenshot `shot` of `entry`, loaded on first use.
    pub fn of_shot(
        &mut self,
        ctx: &egui::Context,
        entry: &Entry,
        shot: usize,
    ) -> Option<egui::TextureHandle> {
        let path = entry.shots.get(shot)?.source.clone()?;
        self.map
            .entry(path.clone())
            .or_insert_with(|| {
                let img = sc_ocr::load_image(&path).ok()?;
                let size = [img.width() as usize, img.height() as usize];
                let color = egui::ColorImage::from_rgb(size, img.as_raw());
                Some(ctx.load_texture(
                    path.display().to_string(),
                    color,
                    egui::TextureOptions::LINEAR,
                ))
            })
            .clone()
    }
}
