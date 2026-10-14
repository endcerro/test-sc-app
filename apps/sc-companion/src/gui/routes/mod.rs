//! Routes pane, ported from the hauling planner's setup bar and results column: plans the
//! contracts saved in the Contracts pane (board order, so `#N` matches) with
//! [`sc_planner::plan`] on a background thread, and shows the ranked routes.

mod format;
mod results;
mod setup;

use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use eframe::egui::{self, Color32, Key, Modifiers, RichText, Stroke, Ui};
use sc_contracts::Contract;
use sc_data::LoadedUniverse;
use sc_planner::{PlanError, PlanOutput, Settings, Ship};

use super::contracts::ContractsPane;
use crate::contracts::review;
use crate::gui::theme;
use crate::routes::RouteSetup;

/// Widest the pane gets, so cards stay readable on large windows.
const MAX_W: f32 = 1100.0;

/// What a plan is made from.
#[derive(Clone)]
struct Input {
    contracts: Vec<Contract>,
    ship: Ship,
    settings: Settings,
}

struct Planned {
    out: PlanOutput,
    input: Input,
}

/// What the Contracts pane holds, summarised for the header; recomputed when it changes.
#[derive(Default)]
struct Board {
    contracts: Vec<Contract>,
    /// Fields still to check (the Contracts pane's "Next to check").
    to_check: usize,
    /// Contract numbers that look like the same contract.
    duplicates: Vec<(usize, usize)>,
}

pub struct RoutesPane {
    data: Arc<LoadedUniverse>,
    dir: PathBuf,
    setup: RouteSetup,
    /// The setup as last saved.
    saved: RouteSetup,
    planning: Option<Receiver<(Result<PlanOutput, PlanError>, Input)>>,
    plan: Option<Planned>,
    /// Rank of the route whose stops are shown.
    expanded: Option<usize>,
    error: Option<String>,
    board: Board,
}

impl RoutesPane {
    /// `dir` is where `planner.json` is kept (the contracts directory).
    pub fn new(data: Arc<LoadedUniverse>, dir: PathBuf) -> Self {
        let (setup, warning) = RouteSetup::load(&dir);
        Self {
            data,
            dir,
            saved: setup.clone(),
            setup,
            planning: None,
            plan: None,
            expanded: None,
            error: warning.map(|w| format!("Couldn't read the planner settings: {w}")),
            board: Board::default(),
        }
    }

    pub fn show(&mut self, ctx: &egui::Context, contracts: &ContractsPane) {
        self.poll();
        self.refresh_board(contracts);
        let plan_key = ctx.input_mut(|i| i.consume_key(Modifiers::CTRL, Key::Enter));
        let mut plan = false;
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("routes-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(MAX_W.min(ui.available_width()));
                        let busy = self.planning.is_some();
                        plan = setup::show(
                            ui,
                            &mut self.setup,
                            contracts.places(),
                            &self.data.universe,
                            busy,
                        );
                        ui.add_space(6.0);
                        self.error_box(ui);
                        self.board_line(ui);
                        ui.add_space(6.0);
                        self.results(ui);
                    });
            });
        if plan || plan_key {
            self.start(ctx, contracts);
        }
        self.save_if_changed();
    }

    fn poll(&mut self) {
        let Some(rx) = &self.planning else {
            return;
        };
        let Ok((result, input)) = rx.try_recv() else {
            return;
        };
        self.planning = None;
        match result {
            Ok(out) => {
                self.expanded = None;
                self.plan = Some(Planned { out, input });
            }
            Err(e) => self.error = Some(e.to_string()),
        }
    }

    fn refresh_board(&mut self, contracts: &ContractsPane) {
        let entries = contracts.store().board();
        if entries.len() == self.board.contracts.len()
            && entries
                .iter()
                .zip(&self.board.contracts)
                .all(|(e, c)| e.contract == *c)
        {
            return;
        }
        let number = |id: u64| entries.iter().position(|e| e.id == id).map_or(0, |i| i + 1);
        let mut duplicates: Vec<(usize, usize)> = review::duplicates(contracts.store().entries())
            .into_iter()
            .map(|(a, b)| (number(a).min(number(b)), number(a).max(number(b))))
            .collect();
        duplicates.sort_unstable();
        self.board = Board {
            contracts: entries.iter().map(|e| e.contract.clone()).collect(),
            to_check: contracts.fields_to_check(),
            duplicates,
        };
    }

    /// Plans the board on a thread; the result arrives in [`Self::poll`].
    fn start(&mut self, ctx: &egui::Context, contracts: &ContractsPane) {
        if self.planning.is_some() {
            return;
        }
        if self.board.contracts.is_empty() {
            self.error = Some("No contracts yet: add some on the Contracts pane first.".into());
            return;
        }
        self.error = None;
        let input = Input {
            contracts: self.board.contracts.clone(),
            ship: self.setup.ship().clone(),
            settings: self.setup.settings.clone(),
        };
        let (tx, rx) = channel();
        let (data, places, ctx) = (self.data.clone(), contracts.places().clone(), ctx.clone());
        std::thread::spawn(move || {
            let u = &data.universe;
            let result = sc_planner::plan(
                &input.contracts,
                u,
                |name| places.resolve(u, name),
                &input.ship,
                &input.settings,
            );
            // The pane may be gone (app closing): nothing to do then.
            let _ = tx.send((result, input));
            ctx.request_repaint();
        });
        self.planning = Some(rx);
    }

    fn save_if_changed(&mut self) {
        if self.setup == self.saved {
            return;
        }
        match self.setup.save(&self.dir) {
            Ok(()) => self.saved = self.setup.clone(),
            Err(e) => {
                self.error = Some(format!("Couldn't save the planner settings: {e:#}"));
                // Don't retry every frame.
                self.saved = self.setup.clone();
            }
        }
    }

    // ---- drawing ---------------------------------------------------------------------

    fn error_box(&mut self, ui: &mut Ui) {
        let Some(e) = self.error.clone() else {
            return;
        };
        egui::Frame::new()
            .fill(Color32::from_rgb(0x2a, 0x15, 0x17))
            .stroke(Stroke::new(1.0_f32, theme::BAD))
            .corner_radius(theme::RADIUS)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(e);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            });
        ui.add_space(6.0);
    }

    /// What will be planned, and what may make the plan wrong.
    fn board_line(&self, ui: &mut Ui) {
        let b = &self.board;
        ui.horizontal_wrapped(|ui| {
            let n = b.contracts.len();
            let plural = if n == 1 { "" } else { "s" };
            ui.label(
                RichText::new(format!("{n} contract{plural} on the Contracts pane"))
                    .color(theme::MUTED),
            );
            if b.to_check > 0 {
                ui.label(
                    RichText::new(format!(
                        "\u{b7} {} field(s) to check there (F8)",
                        b.to_check
                    ))
                    .color(theme::WARN),
                );
            }
            for (a, c) in &b.duplicates {
                ui.label(
                    RichText::new(format!("\u{b7} #{a} and #{c} look like duplicates"))
                        .color(theme::WARN),
                );
            }
        });
        if let Some(Planned { input: p, .. }) = &self.plan {
            let mut changed = Vec::new();
            if p.contracts != b.contracts {
                changed.push("the contracts");
            }
            if p.ship != *self.setup.ship() {
                changed.push("the ship");
            }
            if p.settings != self.setup.settings {
                changed.push("the settings");
            }
            if !changed.is_empty() {
                ui.label(
                    RichText::new(format!(
                        "{} changed since this plan: plan again (Ctrl+Enter)",
                        capitalised(&changed.join(" and "))
                    ))
                    .color(theme::WARN),
                );
            }
        }
        if let Some(v) = self
            .plan
            .as_ref()
            .and_then(|p| p.out.data_version.as_deref())
        {
            ui.label(
                RichText::new(format!("Universe data: game {v}"))
                    .color(theme::MUTED)
                    .size(12.0),
            );
        }
    }

    fn results(&mut self, ui: &mut Ui) {
        if self.planning.is_some() {
            results::empty(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.spinner();
                    ui.label(RichText::new("Planning...").color(theme::MUTED));
                });
            });
            return;
        }
        let Some(p) = &self.plan else {
            results::empty(ui, |ui| {
                ui.heading(RichText::new("Recommended routes").size(15.0).strong());
                ui.add_space(4.0);
                for line in [
                    "1. Add contracts on the Contracts pane (drop screenshots or paste them), and check them.",
                    "2. Set your ship and where you are, above.",
                    "3. Press Plan routes, or Ctrl+Enter.",
                ] {
                    ui.label(RichText::new(line).color(theme::MUTED));
                }
                ui.add_space(4.0);
                ui.label(
                    RichText::new("Travel times and capacities are estimates.")
                        .color(theme::MUTED)
                        .size(12.5),
                );
            });
            return;
        };
        results::show(ui, &p.out, &p.input.ship, &mut self.expanded);
    }
}

fn capitalised(s: &str) -> String {
    let mut chars = s.chars();
    chars
        .next()
        .map_or_else(String::new, |c| c.to_uppercase().chain(chars).collect())
}
