//! Contracts pane (a temporary view): drop hauling contract screenshots, they are OCR'd in
//! the background, parsed into flows, saved, and listed as editable cards in a column
//! (newest first).

mod card;
mod draft;
mod format;
mod worker;

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use card::{Action, Captures, Card, Resolver};
use draft::Draft;
use eframe::egui::{self, RichText, Sense, Stroke};
use sc_data::LoadedUniverse;
use worker::{Outcome, Worker};

use crate::contracts::reader::{EXTENSIONS, NO_OCR, file_name, is_screenshot};
use crate::contracts::store::Added;
use crate::contracts::{ContractReader, ContractStore, ShotText};

/// Most messages kept under the drop box.
const MAX_MESSAGES: usize = 6;

#[derive(Debug)]
struct Message {
    text: String,
    error: bool,
}

pub struct ContractsPane {
    data: Arc<LoadedUniverse>,
    store: ContractStore,
    /// `None` when this build has no OCR.
    worker: Option<Worker>,
    messages: Vec<Message>,
    /// Editable copies of the contracts, by entry id.
    drafts: HashMap<u64, Draft>,
    /// Place name -> universe path, see [`Resolver`].
    paths: HashMap<String, Option<String>>,
    captures: Captures,
    confirm_remove: Option<u64>,
    confirm_clear: bool,
}

impl ContractsPane {
    pub fn new(ctx: &egui::Context, data: Arc<LoadedUniverse>, dir: PathBuf) -> Self {
        let (store, warnings) = ContractStore::open(dir);
        let messages = warnings
            .into_iter()
            .map(|w| Message {
                text: format!("Skipped a saved contract: {w}"),
                error: true,
            })
            .collect();
        Self {
            data,
            store,
            worker: ContractReader::available().then(|| Worker::start(ctx)),
            messages,
            drafts: HashMap::new(),
            paths: HashMap::new(),
            captures: Captures::new(),
            confirm_remove: None,
            confirm_clear: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.take_dropped_files(ctx);
        self.poll_worker();
        egui::CentralPanel::default().show(ctx, |ui| {
            self.header(ui);
            self.drop_box(ui);
            self.message_lines(ui);
            ui.add_space(6.0);
            self.column(ui);
        });
    }

    fn take_dropped_files(&mut self, ctx: &egui::Context) {
        let paths: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        self.submit(paths);
    }

    fn submit(&mut self, paths: Vec<PathBuf>) {
        for path in paths {
            if !is_screenshot(&path) {
                self.message(
                    format!(
                        "Skipped {}: not a PNG, JPEG or WebP image",
                        file_name(&path)
                    ),
                    true,
                );
                continue;
            }
            match &mut self.worker {
                Some(worker) => worker.submit(path),
                None => {
                    self.message(NO_OCR.to_owned(), true);
                    return;
                }
            }
        }
    }

    fn poll_worker(&mut self) {
        let Some(worker) = &mut self.worker else {
            return;
        };
        for outcome in worker.poll() {
            match outcome {
                Outcome::Read(shot) => self.add(shot),
                Outcome::Failed { path, error } => {
                    self.message(format!("{}: {error}", file_name(&path)), true);
                }
            }
        }
    }

    fn add(&mut self, shot: ShotText) {
        let name = shot.source.as_deref().map(file_name).unwrap_or_default();
        match self.store.add_shot(shot, &self.data.universe) {
            Ok(added) => {
                let id = added.id();
                if let (Added::Merged(_), Some(e)) = (added, self.store.get(id)) {
                    let title = e.contract.title.clone().unwrap_or_default();
                    self.message(format!("{name} merged into \"{title}\""), false);
                }
            }
            Err(e) => self.message(format!("{name}: could not save: {e:#}"), true),
        }
    }

    fn message(&mut self, text: String, error: bool) {
        if error {
            tracing::warn!("{text}");
        }
        self.messages.push(Message { text, error });
        let excess = self.messages.len().saturating_sub(MAX_MESSAGES);
        self.messages.drain(..excess);
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.horizontal(|ui| {
            ui.heading("Hauling contracts");
            ui.label(RichText::new(format!("{} saved", self.store.entries().len())).weak())
                .on_hover_text(self.store.dir().display().to_string());
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if self.store.entries().is_empty() {
                    return;
                }
                if !self.confirm_clear {
                    self.confirm_clear = ui.button("Clear all").clicked();
                    return;
                }
                if ui.button("Cancel").clicked() {
                    self.confirm_clear = false;
                }
                if ui
                    .button(RichText::new("Remove all").color(ui.visuals().error_fg_color))
                    .clicked()
                {
                    self.confirm_clear = false;
                    self.drafts.clear();
                    self.captures.clear();
                    if let Err(e) = self.store.clear() {
                        self.message(format!("Could not remove every contract: {e:#}"), true);
                    }
                }
                ui.label(format!(
                    "Remove all {} contracts?",
                    self.store.entries().len()
                ));
            });
        });
        ui.add_space(4.0);
    }

    fn drop_box(&mut self, ui: &mut egui::Ui) {
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let visuals = ui.visuals();
        let stroke = if hovering {
            Stroke::new(2.0_f32, visuals.selection.stroke.color)
        } else {
            Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color)
        };
        let fill = if hovering {
            visuals.selection.bg_fill.gamma_multiply(0.25)
        } else {
            visuals.extreme_bg_color
        };
        let response = egui::Frame::group(ui.style())
            .stroke(stroke)
            .fill(fill)
            .inner_margin(18)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    if self.worker.is_none() {
                        ui.label(RichText::new(NO_OCR).color(ui.visuals().warn_fg_color));
                        return;
                    }
                    let text = if hovering {
                        "Release to read the screenshots"
                    } else {
                        "Drop contract screenshots here"
                    };
                    ui.label(RichText::new(text).strong().size(16.0));
                    ui.label(
                        RichText::new(
                            "or click to browse (PNG, JPEG, WebP; Contracts > Offers screen)",
                        )
                        .weak(),
                    );
                    if let Some(w) = self.worker.as_ref().filter(|w| w.pending() > 0) {
                        ui.add_space(6.0);
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(w.status().unwrap_or("Waiting..."));
                            if w.pending() > 1 {
                                ui.label(
                                    RichText::new(format!("({} queued)", w.pending() - 1)).weak(),
                                );
                            }
                        });
                    }
                });
            })
            .response
            .interact(Sense::click());
        if self.worker.is_some() {
            let response = response.on_hover_cursor(egui::CursorIcon::PointingHand);
            if response.clicked()
                && let Some(paths) = rfd::FileDialog::new()
                    .set_title("Contract screenshots")
                    .add_filter("Screenshots", &EXTENSIONS)
                    .pick_files()
            {
                self.submit(paths);
            }
        }
    }

    fn message_lines(&mut self, ui: &mut egui::Ui) {
        let mut dismiss = None;
        for (i, m) in self.messages.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("x").on_hover_text("Dismiss").clicked() {
                    dismiss = Some(i);
                }
                let color = if m.error {
                    ui.visuals().error_fg_color
                } else {
                    ui.visuals().weak_text_color()
                };
                ui.label(RichText::new(&m.text).color(color));
            });
        }
        if let Some(i) = dismiss {
            self.messages.remove(i);
        }
    }

    fn column(&mut self, ui: &mut egui::Ui) {
        if self.store.entries().is_empty() {
            ui.add_space(20.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("No contracts yet.").weak());
            });
            return;
        }
        let mut actions: Vec<(u64, Action)> = Vec::new();
        let Self {
            data,
            store,
            drafts,
            paths,
            captures,
            confirm_remove,
            ..
        } = self;
        let mut resolver = Resolver {
            universe: &data.universe,
            cache: paths,
        };
        let total = store.entries().len();
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for (i, entry) in store.entries().iter().enumerate() {
                    let draft = drafts
                        .entry(entry.id)
                        .or_insert_with(|| Draft::new(&entry.contract));
                    // Changed elsewhere (merged screenshot, re-parse): start over from it.
                    if draft.base != entry.contract {
                        *draft = Draft::new(&entry.contract);
                    }
                    let card = Card {
                        number: total - i,
                        entry,
                        draft,
                        resolver: &mut resolver,
                        captures,
                        confirm_remove: *confirm_remove == Some(entry.id),
                    };
                    actions.extend(card.show(ui).into_iter().map(|a| (entry.id, a)));
                    ui.add_space(8.0);
                }
            });
        for (id, action) in actions {
            self.apply(id, action);
        }
    }

    fn apply(&mut self, id: u64, action: Action) {
        let result = match action {
            Action::Commit => match self.drafts.get_mut(&id) {
                Some(draft) => {
                    let contract = draft.to_contract();
                    let saved = self.store.update(id, contract.clone());
                    if saved.is_ok() {
                        // Keep the draft: blank rows the player just added stay visible.
                        draft.base = contract;
                    }
                    saved
                }
                None => Ok(()),
            },
            Action::AskRemove => {
                self.confirm_remove = Some(id);
                Ok(())
            }
            Action::CancelRemove => {
                self.confirm_remove = None;
                Ok(())
            }
            Action::Remove => {
                self.confirm_remove = None;
                self.drafts.remove(&id);
                self.captures.remove(&id);
                self.store.remove(id)
            }
            Action::Reparse => {
                self.drafts.remove(&id);
                self.store.reparse(id, &self.data.universe)
            }
        };
        if let Err(e) = result {
            self.message(format!("Could not save the contract: {e:#}"), true);
        }
    }
}
