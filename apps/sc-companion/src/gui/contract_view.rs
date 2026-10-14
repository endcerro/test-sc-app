//! A contract shown compact and read-only: a header line (number, title, reward, SCU,
//! status) and a summary of its cargo lines. The Contracts pane shows finalized contracts
//! this way (and the header of every card); it only needs a [`Contract`] and a place lookup,
//! so other panes can show contracts the same way.

use eframe::egui::{self, Align, Color32, Layout, RichText, Sense, Ui};
use sc_contracts::{Contract, Flow, truth};

use crate::contracts::places::PlaceTree;
use crate::contracts::review::Status;
use crate::gui::theme;
use crate::gui::widgets::badge;

/// What the header line shows.
pub struct Header<'a> {
    /// `#N` on the board.
    pub number: usize,
    pub contract: &'a Contract,
    /// `None` hides the badge.
    pub status: Option<Status>,
    pub collapsed: bool,
}

/// The header line, with `right` drawn at the right end (right to left: the first widget
/// added is the rightmost). Returns whether the chevron or the title was clicked (fold or
/// unfold the card).
pub fn header(ui: &mut Ui, h: &Header, right: impl FnOnce(&mut Ui)) -> bool {
    let c = h.contract;
    let ctx = ui.ctx().clone();
    let mut toggled = false;
    ui.horizontal(|ui| {
        let chevron = if h.collapsed {
            theme::sym(&ctx, "\u{25b8}", ">")
        } else {
            theme::sym(&ctx, "\u{25be}", "v")
        };
        let tip = if h.collapsed { "Unfold" } else { "Fold" };
        toggled |= ui
            .add(
                egui::Button::new(RichText::new(chevron).size(14.0).color(theme::MUTED))
                    .frame(false)
                    .min_size(egui::vec2(18.0, 22.0)),
            )
            .on_hover_text(tip)
            .clicked();
        ui.label(
            RichText::new(format!("#{}", h.number))
                .color(theme::MUTED)
                .strong(),
        );
        ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
            right(ui);
            if let Some(s) = h.status {
                status_badge(ui, s);
            }
            if c.flows.iter().any(|f| f.loaded) {
                let loaded = c.flows.iter().filter(|f| f.loaded).count();
                ui.label(
                    RichText::new(format!("{loaded}/{} loaded", c.flows.len()))
                        .color(theme::MUTED)
                        .size(12.5),
                );
            }
            ui.label(
                RichText::new(format!("{} SCU", c.total_scu()))
                    .color(theme::CARGO)
                    .strong(),
            );
            ui.label(
                RichText::new(c.reward.map_or_else(
                    || "? aUEC".to_owned(),
                    |r| format!("{} aUEC", truth::thousands(r)),
                ))
                .color(if c.reward.is_some() {
                    theme::REWARD
                } else {
                    theme::BAD
                })
                .strong(),
            );
            ui.with_layout(Layout::left_to_right(Align::Center), |ui| {
                let title = c.title.as_deref().unwrap_or("(no title)");
                toggled |= ui
                    .add(
                        egui::Label::new(RichText::new(title).strong().size(14.5))
                            .truncate()
                            .sense(Sense::click()),
                    )
                    .on_hover_text(format!("{title}\n({} the card)", tip.to_lowercase()))
                    .clicked();
            });
        });
    });
    toggled
}

pub fn status_badge(ui: &mut Ui, s: Status) {
    let (text, color, border) = match s {
        Status::Ready => (
            "Ready".to_owned(),
            theme::GOOD,
            Color32::from_rgb(0x2f, 0x5b, 0x3c),
        ),
        Status::ToCheck(n) => (
            format!("{n} to check"),
            theme::WARN,
            Color32::from_rgb(0x6b, 0x4d, 0x2b),
        ),
        Status::Blocked(n) => (
            format!("{n} to fix"),
            theme::BAD,
            Color32::from_rgb(0x6b, 0x2f, 0x33),
        ),
    };
    let tip = match s {
        Status::Ready => "Nothing left to check",
        Status::ToCheck(_) => "Doubtful reads: check them against the screenshot",
        Status::Blocked(_) => "Missing or unknown values: the planner can't use them",
    };
    ui.scope(|ui| badge(ui, &text, color, border))
        .response
        .on_hover_text(tip);
}

/// Contractor, time on the board, largest box and reputation, the known ones only:
/// `Covalex · on board 1h 4m · max box 4 SCU · rep 120`.
pub fn details(c: &Contract) -> String {
    let mut parts = Vec::new();
    if let Some(x) = c.contractor.as_deref().filter(|s| !s.is_empty()) {
        parts.push(x.to_owned());
    }
    if let Some(x) = c.availability.as_deref().filter(|s| !s.is_empty()) {
        parts.push(format!("on board {x}"));
    }
    if let Some(x) = c.max_box {
        parts.push(format!("max box {x} SCU"));
    }
    if let Some(x) = c.rep {
        parts.push(format!("rep {x}"));
    }
    parts.join(" \u{b7} ")
}

/// Colour and hover text of a place by where its name resolved.
fn place_look(tree: &PlaceTree) -> (Color32, String) {
    match tree {
        PlaceTree::Empty => (theme::BAD, "Missing place".to_owned()),
        PlaceTree::Known { path, no_distance } => {
            let mut tip = path.join(" \u{203a} ");
            if *no_distance {
                tip.push_str("\n(no distance data)");
            }
            let color = if *no_distance {
                theme::WARN
            } else {
                theme::TEXT
            };
            (color, tip)
        }
        PlaceTree::Ambiguous(paths) => (
            theme::WARN,
            format!(
                "Several places have this name:\n{}",
                paths
                    .iter()
                    .map(|p| p.join(" \u{203a} "))
                    .collect::<Vec<_>>()
                    .join("\n")
            ),
        ),
        PlaceTree::Unknown => (theme::BAD, "Unknown place".to_owned()),
    }
}

/// Column widths of the cargo lines: (SCU, cargo, route, loaded).
fn columns(width: f32) -> (f32, f32, f32, f32) {
    let (scu, loaded) = (58.0, 64.0);
    let cargo = ((width - scu - loaded) * 0.28).clamp(80.0, 200.0);
    let route = (width - scu - loaded - cargo - 18.0).max(60.0);
    (scu, cargo, route, loaded)
}

/// The details line and one row per cargo line: `12 SCU  Gold  A, B → C (4), D (8)`, places
/// coloured by how they resolved (hover for where). `tree` looks a place name up. With
/// `loaded_editable` the loaded boxes can be ticked; returns the (line, loaded) ticked.
pub fn body(
    ui: &mut Ui,
    c: &Contract,
    tree: &mut dyn FnMut(&str) -> PlaceTree,
    loaded_editable: bool,
) -> Option<(usize, bool)> {
    ui.scope(|ui| {
        // A summary to read, not text to select; rows close together.
        ui.style_mut().interaction.selectable_labels = false;
        ui.spacing_mut().item_spacing.y = 2.0;
        rows(ui, c, tree, loaded_editable)
    })
    .inner
}

fn rows(
    ui: &mut Ui,
    c: &Contract,
    tree: &mut dyn FnMut(&str) -> PlaceTree,
    loaded_editable: bool,
) -> Option<(usize, bool)> {
    let details = details(c);
    if !details.is_empty() {
        ui.label(RichText::new(details).color(theme::MUTED).size(12.5));
        ui.add_space(2.0);
    }
    let (scu_w, cargo_w, route_w, loaded_w) = columns(ui.available_width());
    let mut ticked = None;
    for (k, flow) in c.flows.iter().enumerate() {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            cell(ui, scu_w, |ui| {
                ui.label(
                    RichText::new(format!("{} SCU", flow.scu()))
                        .color(theme::CARGO)
                        .strong(),
                );
            });
            cell(ui, cargo_w, |ui| {
                let (text, color) = if flow.commodity.trim().is_empty() {
                    ("(cargo?)".to_owned(), theme::BAD)
                } else if flow.restricted {
                    (flow.display_commodity(), theme::WARN)
                } else {
                    (flow.commodity.clone(), theme::TEXT)
                };
                let r = ui.add(egui::Label::new(RichText::new(text).color(color)).truncate());
                if flow.restricted {
                    r.on_hover_text("Restricted cargo");
                }
            });
            cell(ui, route_w, |ui| route(ui, flow, tree));
            cell(ui, loaded_w, |ui| {
                let mut on = flow.loaded;
                let r = ui
                    .add_enabled(loaded_editable, egui::Checkbox::new(&mut on, "loaded"))
                    .on_hover_text("Already on board");
                if r.changed() {
                    ticked = Some((k, on));
                }
            });
        });
    }
    ticked
}

fn cell(ui: &mut Ui, width: f32, add: impl FnOnce(&mut Ui)) {
    ui.allocate_ui_with_layout(
        egui::vec2(width, 22.0),
        Layout::left_to_right(Align::Center),
        |ui| {
            ui.set_width(width);
            add(ui);
        },
    );
}

/// `A, B → C (4), D (8)`: pickups, then drop-offs (with their SCU when several).
fn route(ui: &mut Ui, flow: &Flow, tree: &mut dyn FnMut(&str) -> PlaceTree) {
    let arrow = theme::sym(ui.ctx(), "\u{2192}", "->").to_owned();
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        let mut place = |ui: &mut Ui, p: &sc_contracts::Place| {
            let (color, tip) = place_look(&tree(&p.name));
            let name = if p.name.trim().is_empty() {
                "?".to_owned()
            } else {
                p.to_string()
            };
            ui.label(RichText::new(name).color(color))
                .on_hover_text(tip);
        };
        if flow.pickups.is_empty() {
            ui.label(RichText::new("(pickup?)").color(theme::BAD));
        }
        for (j, p) in flow.pickups.iter().enumerate() {
            if j > 0 {
                ui.label(RichText::new(", ").color(theme::MUTED));
            }
            place(ui, p);
        }
        ui.label(RichText::new(format!("  {arrow}  ")).color(theme::MUTED));
        if flow.dropoffs.is_empty() {
            ui.label(RichText::new("(drop-off?)").color(theme::BAD));
        }
        for (j, d) in flow.dropoffs.iter().enumerate() {
            if j > 0 {
                ui.label(RichText::new(", ").color(theme::MUTED));
            }
            place(ui, &d.place);
            if flow.dropoffs.len() > 1 {
                ui.label(RichText::new(format!(" ({})", d.scu)).color(theme::CARGO));
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn details_skip_unknown_values() {
        let mut c = Contract {
            contractor: Some("Covalex".into()),
            availability: Some("1h 4m".into()),
            max_box: Some(4),
            ..Contract::default()
        };
        assert_eq!(
            details(&c),
            "Covalex \u{b7} on board 1h 4m \u{b7} max box 4 SCU"
        );
        c.contractor = Some(String::new());
        c.availability = None;
        c.rep = Some(120);
        assert_eq!(details(&c), "max box 4 SCU \u{b7} rep 120");
        assert_eq!(details(&Contract::default()), "");
    }

    #[test]
    fn columns_fit_the_width() {
        let (a, b, c, d) = columns(900.0);
        assert!(a + b + c + d + 18.0 <= 900.0 + 0.5);
        assert!(c > b);
    }
}
