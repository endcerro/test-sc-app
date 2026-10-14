//! Ranked route cards, ported from the hauling planner: headline numbers, the trip strip
//! (stops spaced by time), the cargo strip, warnings and the stop table.

use eframe::egui::{self, Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui};
use sc_planner::{CargoMove, ContractStatus, Fit, LegKind, PlanOutput, RouteOut, Ship, StepOut};

use super::format::{mins, money, num};
use crate::gui::theme;
use crate::gui::widgets::{badge, dashed_segment, small_button};

/// Left and right margin of the strips, so the end labels fit.
const PAD: f32 = 24.0;

pub fn empty(ui: &mut Ui, add: impl FnOnce(&mut Ui)) {
    egui::Frame::new()
        .stroke(Stroke::new(1.0_f32, theme::LINE))
        .corner_radius(theme::RADIUS)
        .inner_margin(16)
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            add(ui);
        });
}

/// The routes of `plan`, planned with `ship`. `expanded` is the rank whose stops are shown.
pub fn show(ui: &mut Ui, plan: &PlanOutput, ship: &Ship, expanded: &mut Option<usize>) {
    ui.heading(RichText::new("Recommended routes").size(15.0).strong());
    ui.label(
        RichText::new(format!("{} \u{b7} from {}", plan.goal, plan.start)).color(theme::MUTED),
    );
    if !plan.issues.is_empty() {
        egui::Frame::new()
            .stroke(Stroke::new(1.0_f32, theme::WARN))
            .corner_radius(theme::RADIUS)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                for i in &plan.issues {
                    ui.label(RichText::new(i).color(theme::WARN).size(13.0));
                }
            });
    }
    if plan.routes.is_empty() {
        empty(ui, |ui| {
            ui.label(
                RichText::new(
                    "No route found. Check the contracts left out below, or loosen the goal.",
                )
                .color(theme::MUTED),
            );
        });
    }
    for r in &plan.routes {
        route_card(ui, r, plan, ship, expanded);
        ui.add_space(8.0);
    }
    let left_out: Vec<_> = plan
        .contracts
        .iter()
        .filter(|c| c.status != ContractStatus::TopRoute)
        .collect();
    if !left_out.is_empty() {
        theme::card().show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new("Not in the top route").strong());
            for c in left_out {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!("#{}", c.contract))
                            .color(theme::MUTED)
                            .size(13.0),
                    );
                    ui.label(RichText::new(c.title.as_deref().unwrap_or("(no title)")).size(13.0));
                    let color = match c.status {
                        ContractStatus::LeftOut(_) => theme::WARN,
                        _ => theme::MUTED,
                    };
                    ui.label(
                        RichText::new(format!(": {}", c.status))
                            .color(color)
                            .size(13.0),
                    );
                });
            }
        });
    }
}

fn route_card(
    ui: &mut Ui,
    r: &RouteOut,
    plan: &PlanOutput,
    ship: &Ship,
    expanded: &mut Option<usize>,
) {
    let ctx = ui.ctx().clone();
    let top = r.rank == 1;
    let frame = theme::card()
        .inner_margin(egui::Margin::symmetric(14, 12))
        .stroke(Stroke::new(
            if top { 2.0_f32 } else { 1.0 },
            if top { theme::ACCENT } else { theme::LINE },
        ));
    frame.show(ui, |ui| {
        ui.set_width(ui.available_width());
        ui.horizontal(|ui| {
            ui.label(
                RichText::new(format!("#{}", r.rank))
                    .strong()
                    .color(if top { theme::ACCENT } else { theme::MUTED }),
            );
            ui.label(RichText::new(&r.label).strong());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let (text, color, border) = match r.fit {
                    Fit::Comfortable => (
                        "Comfortable fit",
                        theme::GOOD,
                        Color32::from_rgb(0x2f, 0x5b, 0x3c),
                    ),
                    Fit::Tight => (
                        theme::sym(&ctx, "\u{25d0} Tight fit", "Tight fit"),
                        theme::WARN,
                        Color32::from_rgb(0x6b, 0x4d, 0x2b),
                    ),
                    Fit::OverEstimate => (
                        theme::sym(&ctx, "\u{25b2} Over estimate", "Over estimate"),
                        theme::WARN,
                        theme::WARN,
                    ),
                    Fit::Unknown => ("Capacity not set", theme::MUTED, theme::LINE),
                };
                badge(ui, text, color, border);
            });
        });
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 22.0;
            stat(ui, &money(r.reward), "aUEC", theme::REWARD, 24.0);
            stat(
                ui,
                &mins(r.total_minutes),
                &format!("{} quantum", mins(r.quantum_minutes)),
                theme::TIME,
                24.0,
            );
            stat(
                ui,
                &money(r.reward_per_hour),
                "aUEC per hour",
                theme::REWARD,
                20.0,
            );
            stat(ui, &r.stops.to_string(), "stops", theme::TEXT, 24.0);
            stat(
                ui,
                &r.contracts.len().to_string(),
                "contracts",
                theme::TEXT,
                24.0,
            );
        });
        trip_strip(ui, r, &plan.start);
        cargo_strip(ui, r, ship);
        let reward_of = |c: usize| {
            plan.contracts
                .iter()
                .find(|x| x.contract == c)
                .and_then(|x| x.reward)
                .map_or_else(|| "?".to_owned(), money)
        };
        let accept = r
            .contracts
            .iter()
            .map(|&c| format!("#{c} {}", reward_of(c)))
            .collect::<Vec<_>>()
            .join(" \u{b7} ");
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 0.0;
            ui.label(RichText::new("Accept: ").color(theme::MUTED).size(13.0));
            ui.label(RichText::new(accept).color(theme::REWARD).size(13.0));
        });
        let warn = theme::sym(&ctx, "\u{26a0} ", "! ");
        for w in &r.warnings {
            ui.label(
                RichText::new(format!("{warn}{w}"))
                    .color(theme::WARN)
                    .size(13.0),
            );
        }
        let open = *expanded == Some(r.rank);
        if small_button(ui, if open { "Hide stops" } else { "Show stops" }).clicked() {
            *expanded = if open { None } else { Some(r.rank) };
        }
        if open {
            stop_table(ui, r);
        }
    });
}

fn stat(ui: &mut Ui, value: &str, key: &str, color: Color32, size: f32) {
    ui.vertical(|ui| {
        ui.spacing_mut().item_spacing.y = 0.0;
        ui.label(RichText::new(value).size(size).strong().color(color));
        ui.label(RichText::new(key).size(11.5).color(theme::MUTED));
    });
}

fn moves_text(v: &[CargoMove]) -> String {
    v.iter()
        .map(|m| {
            format!(
                "#{} {}{} {}{}",
                m.contract,
                if m.restricted { "[!] " } else { "" },
                m.commodity,
                num(m.scu),
                if m.estimated_split { "~" } else { "" }
            )
        })
        .collect::<Vec<_>>()
        .join(", ")
}

/// `name` cut to `n` characters, ending with `ellipsis` when cut.
fn short(name: &str, n: usize, ellipsis: &str) -> String {
    if name.chars().count() <= n {
        name.to_owned()
    } else {
        format!(
            "{}{ellipsis}",
            name.chars().take(n.saturating_sub(1)).collect::<String>()
        )
    }
}

fn distance(st: &StepOut) -> String {
    st.travel
        .distance
        .map(|d| format!(" \u{b7} {d}"))
        .unwrap_or_default()
}

fn travel_tip(st: &StepOut) -> String {
    let what = match st.travel.kind {
        LegKind::Quantum => "Quantum travel",
        LegKind::Jump => "Quantum travel through a jump point",
        LegKind::SameBody => "Same-planet hop (your hop time)",
        LegKind::Missing => "No distance data: rough guess",
        LegKind::SamePlace => "Same place",
    };
    let estimate = if st.travel.estimated {
        " (estimate)"
    } else {
        ""
    };
    format!(
        "{what}: {}{}{estimate}",
        mins(st.travel.minutes),
        distance(st)
    )
}

fn stop_tip(st: &StepOut) -> String {
    let mut tip = format!(
        "{}. {} ({}), arrive at {}",
        st.n,
        st.location,
        st.system,
        mins(st.arrive_minutes)
    );
    if !st.picked.is_empty() {
        tip.push_str(&format!("\nLoad: {}", moves_text(&st.picked)));
    }
    if !st.delivered.is_empty() {
        tip.push_str(&format!("\nDrop: {}", moves_text(&st.delivered)));
    }
    tip
}

/// Start, then every stop, spaced by time. Gaps are travel, small bars are time at the
/// stop; jumps and estimated legs are drawn differently and labelled.
fn trip_strip(ui: &mut Ui, r: &RouteOut, start: &str) {
    let ctx = ui.ctx().clone();
    let ellipsis = theme::sym(&ctx, "\u{2026}", "..");
    let w = ui.available_width();
    let (rect, resp) = ui.allocate_exact_size(egui::vec2(w, 108.0), Sense::hover());
    let p = ui.painter_at(rect.expand(4.0));
    let total = r.total_minutes.max(1.0) as f32;
    let x = |t: f64| rect.left() + PAD + t as f32 / total * (w - 2.0 * PAD);
    let y = |v: f32| rect.top() + v;
    let small = FontId::proportional(12.0);
    let pointer = resp.hover_pos();
    let mut tip: Option<String> = None;

    p.circle_filled(Pos2::new(x(0.0), y(44.0)), 6.0, theme::ACCENT);
    p.text(
        Pos2::new(x(0.0) - 6.0, y(10.0)),
        Align2::LEFT_CENTER,
        short(start, 22, ellipsis),
        small.clone(),
        theme::MUTED,
    );
    if pointer.is_some_and(|q| q.distance(Pos2::new(x(0.0), y(44.0))) < 8.0) {
        tip = Some(format!("Start: {start}"));
    }

    let stop = r.stop_minutes / r.stops.max(1) as f64;
    let mut prev = 0.0;
    let mut stops = Vec::new();
    for (i, st) in r.steps.iter().enumerate() {
        let (a, d) = (st.arrive_minutes, st.arrive_minutes + stop);
        let (x0, x1, xd) = (x(prev), x(a), x(d));
        let pts = [Pos2::new(x0, y(44.0)), Pos2::new(x1, y(44.0))];
        let shapes = match st.travel.kind {
            LegKind::Jump => dashed_segment(pts, Stroke::new(5.0_f32, theme::TIME), 10.0, 4.0),
            LegKind::Missing => dashed_segment(pts, Stroke::new(3.0_f32, theme::BAD), 6.0, 4.0),
            LegKind::SamePlace => vec![Shape::line_segment(pts, Stroke::new(3.0_f32, theme::LINE))],
            _ if st.travel.estimated => {
                dashed_segment(pts, Stroke::new(3.0_f32, theme::TIME), 2.0, 4.0)
            }
            _ => vec![Shape::line_segment(pts, Stroke::new(3.0_f32, theme::TIME))],
        };
        p.extend(shapes);
        let label = match st.travel.kind {
            LegKind::SamePlace => String::new(),
            LegKind::Jump => format!(
                "{}{}",
                mins(st.travel.minutes),
                theme::sym(&ctx, " \u{21c4}", " JP")
            ),
            _ => mins(st.travel.minutes),
        };
        if x1 - x0 > 34.0 && !label.is_empty() {
            p.text(
                Pos2::new((x0 + x1) / 2.0, y(32.0)),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(12.5),
                theme::TIME,
            );
        }
        if pointer
            .is_some_and(|q| q.x > x0 + 10.0 && q.x < x1 - 10.0 && (q.y - y(44.0)).abs() < 14.0)
        {
            tip = Some(travel_tip(st));
        }
        p.rect_filled(
            Rect::from_min_size(Pos2::new(x1, y(40.0)), egui::vec2((xd - x1).max(2.0), 8.0)),
            0.0,
            theme::LINE,
        );
        stops.push((i, st, x1));
        prev = d;
    }
    // Stops on top of the lines.
    for (i, st, x1) in stops {
        let c = Pos2::new(x1, y(44.0));
        let (letter, ring) = match (st.picked.is_empty(), st.delivered.is_empty()) {
            (false, false) => ("L/D", theme::TEXT),
            (false, true) => ("L", theme::CARGO),
            _ => ("D", theme::REWARD),
        };
        p.circle(c, 10.0, theme::PANEL_2, Stroke::new(1.5_f32, ring));
        p.text(
            c,
            Align2::CENTER_CENTER,
            letter,
            FontId::proportional(9.5),
            theme::TEXT,
        );
        let ly = if i % 2 == 0 { 84.0 } else { 100.0 };
        p.line_segment(
            [Pos2::new(x1, y(56.0)), Pos2::new(x1, y(ly - 10.0))],
            Stroke::new(1.0_f32, theme::LINE),
        );
        p.text(
            Pos2::new(x1, y(ly)),
            Align2::CENTER_CENTER,
            short(&st.location, 16, ellipsis),
            small.clone(),
            theme::MUTED,
        );
        if pointer.is_some_and(|q| {
            q.distance(c) < 11.0 || ((q.x - x1).abs() < 40.0 && (q.y - y(ly)).abs() < 8.0)
        }) {
            tip = Some(stop_tip(st));
        }
    }
    if let Some(t) = tip {
        resp.on_hover_text_at_pointer(t);
    }
}

/// Cargo on board along the route, with the estimate and hard maximum marked.
fn cargo_strip(ui: &mut Ui, r: &RouteOut, ship: &Ship) {
    const H: f32 = 56.0;
    let w = ui.available_width();
    let (rect, _) = ui.allocate_exact_size(egui::vec2(w, H), Sense::hover());
    let p = ui.painter_at(rect.expand(2.0));
    let total = r.total_minutes.max(1.0) as f32;
    let x = |t: f64| rect.left() + PAD + t as f32 / total * (w - 2.0 * PAD);
    let start_load: f64 = r.on_board_at_start.iter().map(|m| m.scu).sum();
    let top = [Some(r.peak_scu), ship.capacity_estimate, ship.hard_max]
        .into_iter()
        .flatten()
        .fold(1.0f64, f64::max)
        * 1.1;
    let y = |v: f64| rect.top() + 6.0 + (H - 12.0) * (1.0 - (v / top) as f32);

    let mut pts = vec![Pos2::new(x(0.0), y(start_load))];
    let mut load = start_load;
    for st in &r.steps {
        pts.push(Pos2::new(x(st.arrive_minutes), y(load)));
        load = st.cargo_after;
        pts.push(Pos2::new(x(st.arrive_minutes), y(load)));
    }
    pts.push(Pos2::new(x(f64::from(total)), y(load)));
    // The area under a step line, as one rectangle per flat stretch (convex pieces).
    let fill = theme::CARGO.linear_multiply(0.18);
    for pair in pts.windows(2) {
        if pair[1].x > pair[0].x {
            p.rect_filled(
                Rect::from_min_max(pair[0], Pos2::new(pair[1].x, y(0.0))),
                0.0,
                fill,
            );
        }
    }
    p.add(Shape::line(pts, Stroke::new(2.0_f32, theme::CARGO)));
    let small = FontId::proportional(11.5);
    let (left, right) = (rect.left() + PAD, rect.right() - PAD);
    if let Some(v) = ship.capacity_estimate {
        p.extend(dashed_segment(
            [Pos2::new(left, y(v)), Pos2::new(right, y(v))],
            Stroke::new(1.0_f32, theme::MUTED),
            4.0,
            3.0,
        ));
        p.text(
            Pos2::new(left, y(v) - 3.0),
            Align2::LEFT_BOTTOM,
            format!("est. {}", num(v)),
            small.clone(),
            theme::MUTED,
        );
    }
    if let Some(v) = ship.hard_max {
        p.line_segment(
            [Pos2::new(left, y(v)), Pos2::new(right, y(v))],
            Stroke::new(1.0_f32, theme::BAD),
        );
        p.text(
            Pos2::new(right, y(v) - 3.0),
            Align2::RIGHT_BOTTOM,
            format!("max {}", num(v)),
            small,
            theme::BAD,
        );
    }
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 0.0;
        ui.label(
            RichText::new(format!("Cargo on board, peak {} SCU", num(r.peak_scu)))
                .color(theme::CARGO)
                .size(12.0),
        );
        let mut rest = String::new();
        if let Some(c) = ship.capacity_estimate {
            rest.push_str(&format!(" \u{b7} estimate {}", num(c)));
        }
        if let Some(m) = ship.hard_max {
            rest.push_str(&format!(" \u{b7} hard max {}", num(m)));
        }
        ui.label(RichText::new(rest).color(theme::MUTED).size(12.0));
    });
}

fn stop_table(ui: &mut Ui, r: &RouteOut) {
    let w = ui.available_width();
    let cols = [24.0, w * 0.22, w * 0.2, w * 0.2, w * 0.2, w * 0.1];
    egui::Grid::new(("route-stops", r.rank))
        .num_columns(6)
        .striped(true)
        .spacing([6.0, 6.0])
        .show(ui, |ui| {
            for (h, cw) in ["#", "Stop", "Getting there", "Drop", "Load", "Cargo after"]
                .iter()
                .zip(cols)
            {
                cell(ui, cw, RichText::new(*h).color(theme::MUTED).size(12.5));
            }
            ui.end_row();
            if !r.on_board_at_start.is_empty() {
                cell(ui, cols[0], RichText::new(""));
                cell(
                    ui,
                    cols[1],
                    RichText::new("Already on board")
                        .color(theme::MUTED)
                        .size(13.0),
                );
                cell(ui, cols[2], RichText::new(""));
                cell(ui, cols[3], RichText::new(""));
                cell(
                    ui,
                    cols[4],
                    RichText::new(moves_text(&r.on_board_at_start)).size(13.0),
                );
                let scu: f64 = r.on_board_at_start.iter().map(|m| m.scu).sum();
                cell(
                    ui,
                    cols[5],
                    RichText::new(format!("{} SCU", num(scu)))
                        .color(theme::CARGO)
                        .size(13.0),
                );
                ui.end_row();
            }
            for st in &r.steps {
                let m = mins(st.travel.minutes);
                let travel = match st.travel.kind {
                    LegKind::SamePlace => "same place".to_owned(),
                    LegKind::SameBody => format!("{m} hop (est.)"),
                    LegKind::Jump => format!("{m} via jump point{}", distance(st)),
                    LegKind::Missing => format!("{m} (no distance data)"),
                    LegKind::Quantum => format!("{m}{}", distance(st)),
                };
                cell(ui, cols[0], RichText::new(st.n.to_string()).size(13.0));
                ui.allocate_ui(egui::vec2(cols[1], 0.0), |ui| {
                    ui.set_width(cols[1]);
                    ui.vertical(|ui| {
                        ui.spacing_mut().item_spacing.y = 0.0;
                        ui.add(
                            egui::Label::new(RichText::new(&st.location).strong().size(13.0))
                                .wrap(),
                        );
                        let mut sub = st.system.clone();
                        if st.drop_only {
                            sub.push_str(" (drop-off only)");
                        }
                        ui.label(RichText::new(sub).color(theme::MUTED).size(12.0));
                    });
                });
                cell(ui, cols[2], RichText::new(travel).size(13.0));
                cell(
                    ui,
                    cols[3],
                    RichText::new(moves_text(&st.delivered)).size(13.0),
                );
                cell(
                    ui,
                    cols[4],
                    RichText::new(moves_text(&st.picked)).size(13.0),
                );
                cell(
                    ui,
                    cols[5],
                    RichText::new(format!("{} SCU", num(st.cargo_after)))
                        .color(theme::CARGO)
                        .size(13.0),
                );
                ui.end_row();
            }
        });
}

fn cell(ui: &mut Ui, width: f32, text: RichText) {
    ui.allocate_ui(egui::vec2(width, 0.0), |ui| {
        ui.set_width(width);
        ui.add(egui::Label::new(text).wrap());
    });
}

#[cfg(test)]
mod tests {
    use super::short;

    #[test]
    fn shortens_names() {
        assert_eq!(short("Lorville", 16, ".."), "Lorville");
        assert_eq!(short("Covalex Distribution Centre", 8, ".."), "Covalex..");
    }
}
