//! Setup bar, ported from the hauling planner: ship profile, where you are, time
//! settings, goal and the Plan button.

use eframe::egui::{self, Id, RichText, Ui};
use sc_planner::{Goal, MAX_HELD};
use sc_universe::Universe;

use super::format::{num, opt_num, parse_opt};
use crate::contracts::places::{PlaceTree, Places};
use crate::contracts::review::Mark;
use crate::gui::theme;
use crate::gui::widgets::{
    PLACE_CHOICES, field, labeled, number, place, place_tree, primary_button, small_button,
};
use crate::routes::RouteSetup;

/// Width of the "You are at" field.
const START_W: f32 = 220.0;

/// Draws the bar; true when Plan was pressed.
pub fn show(ui: &mut Ui, s: &mut RouteSetup, places: &Places, u: &Universe, busy: bool) -> bool {
    let mut plan = false;
    theme::card().show(ui, |ui| {
        ui.set_width(ui.available_width());
        ship_row(ui, s);
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            let settings = &mut s.settings;
            labeled(ui, "You are at", |ui| {
                let tree = places.tree(u, &settings.start);
                let mark = if matches!(tree, PlaceTree::Unknown | PlaceTree::Ambiguous(_)) {
                    Mark::Bad
                } else {
                    Mark::None
                };
                let suggest = |q: &str| places.suggest(u, q, PLACE_CHOICES);
                let out = place(
                    ui,
                    Id::new("route-start"),
                    &settings.start,
                    START_W,
                    mark,
                    &suggest,
                );
                if let Some(v) = out.commit {
                    settings.start = v.trim().to_owned();
                }
                place_tree(ui, &tree, START_W);
            });
            labeled(ui, "Time per stop (min)", |ui| {
                if let Some(v) = number(
                    ui,
                    Id::new("route-stop"),
                    &num(settings.stop_minutes),
                    90.0,
                    parse_opt,
                ) {
                    settings.stop_minutes = v;
                }
            })
            .response
            .on_hover_text(
                "Landing, loading and take-off at every stop; also the time to cross a jump point",
            );
            labeled(ui, "Same-planet hop (min)", |ui| {
                if let Some(v) = number(
                    ui,
                    Id::new("route-hop"),
                    &num(settings.hop_minutes),
                    90.0,
                    parse_opt,
                ) {
                    settings.hop_minutes = v;
                }
            })
            .response
            .on_hover_text("Travel between two places on or around the same planet or moon");
            labeled(ui, "Already accepted", |ui| {
                if let Some(v) = number(
                    ui,
                    Id::new("route-accepted"),
                    &settings.accepted.to_string(),
                    70.0,
                    parse_opt,
                ) {
                    settings.accepted = (v as u32).min(MAX_HELD as u32);
                }
            })
            .response
            .on_hover_text(format!(
                "Contracts you already hold in game ({MAX_HELD} at most)"
            ));
            labeled(ui, "Goal", |ui| goal_picker(ui, &mut settings.goal));
            match settings.goal {
                Goal::RatePerHour { min_minutes } => {
                    labeled(ui, "Min session (min)", |ui| {
                        if let Some(v) = number(
                            ui,
                            Id::new("route-min-session"),
                            &num(min_minutes),
                            80.0,
                            parse_opt,
                        ) {
                            settings.goal = Goal::RatePerHour { min_minutes: v };
                        }
                    })
                    .response
                    .on_hover_text("Shorter routes are only shown when nothing longer exists");
                }
                Goal::MaxReward { budget_minutes } => {
                    labeled(ui, "Time budget (min)", |ui| {
                        let parse = |t: &str| parse_opt(t).filter(|v| *v >= 1.0);
                        if let Some(v) = number(
                            ui,
                            Id::new("route-budget"),
                            &num(budget_minutes),
                            80.0,
                            parse,
                        ) {
                            settings.goal = Goal::MaxReward { budget_minutes: v };
                        }
                    });
                }
                Goal::CompleteAll => {}
            }
            ui.vertical(|ui| {
                ui.add_space(16.0);
                let text = if busy { "Planning..." } else { "Plan routes" };
                plan = primary_button(ui, text, !busy)
                    .on_hover_text("Ctrl+Enter")
                    .clicked();
            });
        });
    });
    plan
}

fn ship_row(ui: &mut Ui, s: &mut RouteSetup) {
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 10.0;
        labeled(ui, "Ship", |ui| {
            let names: Vec<String> = s.ships.iter().map(|sh| sh.name.clone()).collect();
            egui::ComboBox::from_id_salt("route-ship")
                .width(140.0)
                .selected_text(s.ship().name.clone())
                .show_ui(ui, |ui| {
                    for (i, n) in names.iter().enumerate() {
                        ui.selectable_value(&mut s.ship, i, n);
                    }
                });
        });
        ui.vertical(|ui| {
            ui.add_space(16.0);
            ui.horizontal(|ui| {
                if small_button(ui, "New ship").clicked() {
                    s.add_ship();
                }
                let delete = egui::Button::new(RichText::new("Delete ship").size(13.0));
                if ui.add_enabled(s.ships.len() > 1, delete).clicked() {
                    s.remove_ship();
                }
            });
        });
        let i = s.ship;
        let ship = s.ship_mut();
        labeled(ui, "Name", |ui| {
            let out = field(
                ui,
                Id::new(("ship-name", i)),
                &ship.name,
                150.0,
                Mark::None,
                None,
            );
            if let Some(v) = out.commit.filter(|v| !v.trim().is_empty()) {
                ship.name = v.trim().to_owned();
            }
        });
        let opt = |t: &str| Some(parse_opt(t));
        labeled(ui, "Est. capacity (SCU)", |ui| {
            if let Some(v) = number(
                ui,
                Id::new(("ship-cap", i)),
                &opt_num(ship.capacity_estimate),
                90.0,
                opt,
            ) {
                ship.capacity_estimate = v;
            }
        })
        .response
        .on_hover_text(
            "Estimated cargo capacity: for the comfortable / tight / over estimate indicator",
        );
        labeled(ui, "Hard max (SCU)", |ui| {
            if let Some(v) = number(
                ui,
                Id::new(("ship-max", i)),
                &opt_num(ship.hard_max),
                90.0,
                opt,
            ) {
                ship.hard_max = v;
            }
        })
        .response
        .on_hover_text("Absolute ceiling: no route ever carries more than this");
        labeled(ui, "Largest box (SCU)", |ui| {
            let value = opt_num(ship.max_box.map(f64::from));
            if let Some(v) = number(ui, Id::new(("ship-box", i)), &value, 90.0, opt) {
                ship.max_box = v.map(|x| x as u32);
            }
        })
        .response
        .on_hover_text("Largest container your cargo grid takes");
        labeled(ui, "QT speed (Mm/s)", |ui| {
            let parse = |t: &str| parse_opt(t).filter(|v| *v > 0.0);
            if let Some(v) = number(
                ui,
                Id::new(("ship-qt", i)),
                &num(ship.qt_speed_mms),
                90.0,
                parse,
            ) {
                ship.qt_speed_mms = v;
            }
        })
        .response
        .on_hover_text("Quantum drive cruise speed in megametres per second");
        labeled(ui, "Spool (s)", |ui| {
            if let Some(v) = number(
                ui,
                Id::new(("ship-spool", i)),
                &num(ship.spool_seconds),
                70.0,
                parse_opt,
            ) {
                ship.spool_seconds = v;
            }
        })
        .response
        .on_hover_text("Quantum drive spool time, for every jump");
    });
}

fn goal_picker(ui: &mut Ui, goal: &mut Goal) {
    const RATE: &str = "Best reward per hour";
    const REWARD: &str = "Most reward in a time budget";
    const ALL: &str = "Shortest time for all";
    let text = match goal {
        Goal::RatePerHour { .. } => RATE,
        Goal::MaxReward { .. } => REWARD,
        Goal::CompleteAll => ALL,
    };
    egui::ComboBox::from_id_salt("route-goal")
        .width(210.0)
        .selected_text(text)
        .show_ui(ui, |ui| {
            let rate = matches!(goal, Goal::RatePerHour { .. });
            let reward = matches!(goal, Goal::MaxReward { .. });
            if ui.selectable_label(rate, RATE).clicked() && !rate {
                *goal = Goal::RatePerHour { min_minutes: 20.0 };
            }
            if ui.selectable_label(reward, REWARD).clicked() && !reward {
                *goal = Goal::MaxReward {
                    budget_minutes: 45.0,
                };
            }
            if ui
                .selectable_label(*goal == Goal::CompleteAll, ALL)
                .clicked()
            {
                *goal = Goal::CompleteAll;
            }
        });
}
