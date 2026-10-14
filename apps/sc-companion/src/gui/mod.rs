//! egui/eframe application: a top bar to switch panes (Map, Contracts), one shown at a time.

mod contracts;
mod loader;
mod map;
pub mod theme;

use anyhow::{Result, anyhow};
use contracts::ContractsPane;
use eframe::egui;
use loader::LoadState;
use map::MapPane;

use crate::SourceArgs;

pub fn run(args: SourceArgs) -> Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("SC Companion")
            .with_inner_size([1400.0, 900.0])
            .with_min_inner_size([800.0, 500.0]),
        ..Default::default()
    };
    eframe::run_native(
        "SC Companion",
        options,
        Box::new(move |cc| Ok(Box::new(CompanionApp::new(&cc.egui_ctx, args)))),
    )
    .map_err(|e| anyhow!("GUI error: {e}"))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pane {
    Map,
    Contracts,
    Trading,
}

impl Pane {
    const ALL: [Self; 3] = [Self::Map, Self::Contracts, Self::Trading];

    fn label(self) -> &'static str {
        match self {
            Self::Map => "Map",
            Self::Contracts => "Contracts",
            Self::Trading => "Trading",
        }
    }

    fn available(self) -> bool {
        matches!(self, Self::Map | Self::Contracts)
    }
}

struct CompanionApp {
    args: SourceArgs,
    state: LoadState,
    pane: Pane,
    map: Option<MapPane>,
    contracts: Option<ContractsPane>,
}

impl CompanionApp {
    fn new(ctx: &egui::Context, args: SourceArgs) -> Self {
        theme::apply(ctx);
        let state = LoadState::start(&args, ctx);
        Self {
            args,
            state,
            pane: Pane::Map,
            map: None,
            contracts: None,
        }
    }

    fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top_bar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                ui.strong("SC Companion");
                ui.separator();
                for pane in Pane::ALL {
                    let button = ui.add_enabled(
                        pane.available(),
                        egui::Button::selectable(self.pane == pane, pane.label()),
                    );
                    let button = button.on_disabled_hover_text("Coming soon");
                    if button.clicked() {
                        self.pane = pane;
                    }
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    self.data_status(ui);
                });
            });
        });
    }

    fn data_status(&self, ui: &mut egui::Ui) {
        let LoadState::Ready(data) = &self.state else {
            return;
        };
        let meta = data.universe.meta();
        if meta.from_cache {
            ui.colored_label(ui.visuals().warn_fg_color, "offline (cached)");
        }
        let revision = meta
            .revision
            .as_deref()
            .map_or("-", |r| &r[..r.len().min(8)]);
        ui.label(format!(
            "game {} · {} @ {} · {} locations",
            meta.game_version.as_deref().unwrap_or("unknown"),
            meta.source,
            revision,
            data.universe.len()
        ));
        if !data.notes.is_empty() || !data.warnings.is_empty() {
            ui.label(format!(
                "{} notes, {} warnings",
                data.notes.len(),
                data.warnings.len()
            ))
            .on_hover_ui(|ui| {
                for note in &data.notes {
                    ui.label(note);
                }
                if !data.warnings.is_empty() {
                    ui.label(format!(
                        "{} data warnings (RUST_LOG=sc_data=debug to list)",
                        data.warnings.len()
                    ));
                }
            });
        }
    }
}

impl eframe::App for CompanionApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.state.poll()
            && let LoadState::Ready(data) = &self.state
        {
            self.map = Some(MapPane::new(data.clone()));
            self.contracts = Some(ContractsPane::new(
                ctx,
                data.clone(),
                self.args.contracts_dir(),
            ));
        }
        // Screenshots dropped on another pane go to the Contracts pane.
        if self.contracts.is_some() && ctx.input(|i| !i.raw.dropped_files.is_empty()) {
            self.pane = Pane::Contracts;
        }

        self.top_bar(ctx);

        match &self.state {
            LoadState::Loading(_) => {
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.centered_and_justified(|ui| {
                        ui.horizontal_centered(|ui| {
                            ui.spinner();
                            ui.label("Loading universe data…");
                        });
                    });
                });
            }
            LoadState::Failed(error) => {
                let mut retry = false;
                egui::CentralPanel::default().show(ctx, |ui| {
                    ui.vertical_centered(|ui| {
                        ui.add_space(ui.available_height() / 3.0);
                        ui.heading("Could not load the universe data");
                        ui.colored_label(ui.visuals().error_fg_color, error);
                        retry = ui.button("Retry").clicked();
                    });
                });
                if retry {
                    self.state = LoadState::start(&self.args, ctx);
                }
            }
            LoadState::Ready(_) => {
                let shown = match self.pane {
                    Pane::Map => self.map.as_mut().map(|map| map.show(ctx)),
                    Pane::Contracts => self.contracts.as_mut().map(|c| c.show(ctx)),
                    Pane::Trading => None,
                };
                if shown.is_none() {
                    egui::CentralPanel::default().show(ctx, |ui| {
                        ui.centered_and_justified(|ui| ui.label("Coming soon"));
                    });
                }
            }
        }
    }
}
