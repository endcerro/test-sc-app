//! Right-hand panel: everything the universe knows about the selected location.

use eframe::egui::{self, RichText};
use sc_universe::{Distance, LocationId, LocationKind, Position, Universe, Vec3};

use super::Action;

pub fn show(
    ui: &mut egui::Ui,
    u: &Universe,
    id: LocationId,
    focus: LocationId,
    measure_from: Option<LocationId>,
    actions: &mut Vec<Action>,
) {
    let l = u.get(id);
    ui.horizontal(|ui| {
        ui.heading(&l.name);
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.small_button("Close").clicked() {
                actions.push(Action::Select(None));
            }
        });
    });
    ui.label(RichText::new(u.path_string(id)).weak());
    ui.horizontal(|ui| {
        if ui.button("Focus").clicked() {
            actions.push(Action::Focus(id));
        }
        if ui
            .button("Measure from here")
            .on_hover_text("Show distances from this location when selecting others")
            .clicked()
        {
            actions.push(Action::MeasureFrom(Some(id)));
        }
    });
    ui.separator();

    egui::Grid::new("inspector_props")
        .num_columns(2)
        .spacing([12.0, 4.0])
        .show(ui, |ui| {
            row(ui, "Kind", format!("{} ({})", l.kind.label(), l.raw_type));
            match l.position {
                Position::Exact(v) => row(ui, "Position", fmt_vec(v)),
                Position::Inherited { from, pos } => row(
                    ui,
                    "Position",
                    format!("{}\n~ inherited from {}", fmt_vec(pos), u.get(from).name),
                ),
                Position::Unknown => row(ui, "Position", "unknown".into()),
            }
            if let Some(v) = l.position.vec() {
                row(ui, "From star", Distance(v.length()).to_string());
            }
            if let Some(r) = l.radius_m {
                row(ui, "Radius", Distance(r).to_string());
            }
            row(ui, "Quantum target", yes_no(l.qt_valid).into());
            row(
                ui,
                "Starmap",
                if l.hidden { "hidden" } else { "visible" }.into(),
            );
            if l.has_commodity_trading() {
                row(ui, "Trading", "commodity terminal".into());
            }
            if let Some(tag) = &l.hierarchy_tag {
                row(ui, "Hierarchy tag", tag.clone());
            }
            if let Some(j) = &l.jurisdiction {
                row(ui, "Jurisdiction", j.clone());
            }
            if let Some(uuid) = l.uuid {
                row(ui, "UUID", uuid.to_string());
            }
        });

    distances(ui, u, id, focus, measure_from, actions);
    jumps(ui, u, id, actions);

    if !l.amenities.is_empty() {
        egui::CollapsingHeader::new(format!("Amenities ({})", l.amenities.len()))
            .default_open(true)
            .show(ui, |ui| {
                for a in &l.amenities {
                    ui.label(&a.display).on_hover_text(&a.key);
                }
            });
    }
    if !l.aliases.is_empty() {
        egui::CollapsingHeader::new(format!("Aliases ({})", l.aliases.len()))
            .default_open(true)
            .show(ui, |ui| {
                for a in &l.aliases {
                    ui.label(format!("{}  ", a.name)).on_hover_text(format!(
                        "{} · source: {}",
                        a.kind.label(),
                        a.source
                    ));
                }
            });
    }
    if !l.children.is_empty() {
        egui::CollapsingHeader::new(format!("Children ({})", l.children.len()))
            .default_open(l.children.len() <= 20)
            .show(ui, |ui| {
                for c in u.children(id) {
                    let text = format!("{}  ·  {}", c.name, c.kind.label());
                    let resp = ui.selectable_label(false, text);
                    if resp.clicked() {
                        actions.push(Action::Select(Some(c.id)));
                    }
                    if resp.double_clicked() {
                        actions.push(Action::Focus(c.id));
                    }
                }
            });
    }
}

fn distances(
    ui: &mut egui::Ui,
    u: &Universe,
    id: LocationId,
    focus: LocationId,
    measure_from: Option<LocationId>,
    actions: &mut Vec<Action>,
) {
    let origin = measure_from.unwrap_or(focus);
    if origin == id || u.get(origin).kind == LocationKind::System {
        if let Some(m) = measure_from {
            measuring_note(ui, u, m, actions);
        }
        return;
    }
    ui.separator();
    if let Some(m) = measure_from {
        measuring_note(ui, u, m, actions);
    }
    let from = &u.get(origin).name;
    match u.route(origin, id) {
        Ok(route) if route.jumps() == 0 => {
            ui.label(format!("Distance from {from}: {}", route.total));
        }
        Ok(route) => {
            ui.label(format!(
                "Route from {from}: {} in-system, {} jump(s)",
                route.total,
                route.jumps()
            ));
        }
        Err(e) => {
            ui.label(RichText::new(format!("No distance from {from}: {e}")).weak());
        }
    }
}

fn measuring_note(ui: &mut egui::Ui, u: &Universe, m: LocationId, actions: &mut Vec<Action>) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("Measuring from {}", u.get(m).name)).weak());
        if ui.small_button("clear").clicked() {
            actions.push(Action::MeasureFrom(None));
        }
    });
}

/// Jump tunnels from this location (or, for a gateway, from its jump points).
fn jumps(ui: &mut egui::Ui, u: &Universe, id: LocationId, actions: &mut Vec<Action>) {
    let ends: Vec<LocationId> = std::iter::once(id)
        .chain(
            u.children(id)
                .filter(|c| c.kind == LocationKind::JumpPoint)
                .map(|c| c.id),
        )
        .collect();
    let mut first = true;
    for link in u.jump_links() {
        let other = if ends.contains(&link.a) {
            link.b
        } else if ends.contains(&link.b) {
            link.a
        } else {
            continue;
        };
        if first {
            ui.separator();
            first = false;
        }
        let other_loc = u.get(other);
        let text = format!(
            "Jump to {} ({})",
            u.get(other_loc.system).name,
            other_loc.name
        );
        if ui.button(text).clicked() {
            actions.push(Action::Select(Some(other)));
            actions.push(Action::Focus(other));
        }
    }
}

fn row(ui: &mut egui::Ui, key: &str, value: String) {
    ui.label(RichText::new(key).weak());
    ui.label(value);
    ui.end_row();
}

fn yes_no(b: bool) -> &'static str {
    if b { "yes" } else { "no" }
}

fn fmt_vec(v: Vec3) -> String {
    format!(
        "x {}  y {}  z {}",
        Distance(v.x),
        Distance(v.y),
        Distance(v.z)
    )
}
