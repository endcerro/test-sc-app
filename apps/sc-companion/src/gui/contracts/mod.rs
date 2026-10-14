//! Contracts pane: drop hauling contract screenshots, they are OCR'd in the background,
//! parsed, saved, and listed as cards in a column (newest first). The selected card's
//! screenshots and raw OCR text show in a side panel.

mod format;
mod worker;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use eframe::egui::{self, Color32, RichText, Sense, Stroke};
use sc_contracts::{HaulingContract, Place};
use sc_data::LoadedUniverse;
use worker::{Outcome, Worker};

use crate::contracts::reader::{EXTENSIONS, NO_OCR, file_name, is_screenshot};
use crate::contracts::store::Added;
use crate::contracts::{ContractReader, ContractStore, Entry, ShotText};

/// Most messages kept under the drop box.
const MAX_MESSAGES: usize = 6;

#[derive(Debug)]
struct Message {
    text: String,
    error: bool,
}

/// Screenshot textures of the selected entry, loaded when it is selected.
struct Preview {
    id: u64,
    /// One per shot: the texture, or why there is none.
    images: Vec<Result<egui::TextureHandle, String>>,
}

pub struct ContractsPane {
    data: Arc<LoadedUniverse>,
    store: ContractStore,
    /// `None` when this build has no OCR.
    worker: Option<Worker>,
    messages: Vec<Message>,
    selected: Option<u64>,
    preview: Option<Preview>,
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
            selected: None,
            preview: None,
            confirm_clear: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.take_dropped_files(ctx);
        self.poll_worker();

        if let Some(id) = self.selected {
            let mut close = false;
            egui::SidePanel::right("contract_details")
                .default_width(460.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| close = self.details(ui, id));
                });
            if close {
                self.selected = None;
                self.preview = None;
            }
        }
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
                    let title = format::title(&e.contract).to_owned();
                    self.message(format!("{name} merged into \"{title}\""), false);
                }
                if self.preview.as_ref().is_some_and(|p| p.id == id) {
                    self.preview = None;
                }
                self.selected = Some(id);
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
                    self.selected = None;
                    self.preview = None;
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
        let mut clicked = None;
        egui::ScrollArea::vertical()
            .auto_shrink(false)
            .show(ui, |ui| {
                for entry in self.store.entries() {
                    let selected = self.selected == Some(entry.id);
                    if card(ui, entry, selected).clicked() {
                        clicked = Some(entry.id);
                    }
                    ui.add_space(6.0);
                }
            });
        if let Some(id) = clicked {
            self.selected = (self.selected != Some(id)).then_some(id);
        }
    }

    /// Side panel of the selected entry. Returns true to close it.
    fn details(&mut self, ui: &mut egui::Ui, id: u64) -> bool {
        let Some(entry) = self.store.get(id).cloned() else {
            return true;
        };
        let mut close = false;
        ui.horizontal(|ui| {
            ui.strong("Screenshots");
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                close = ui.button("Close").clicked();
                if ui.button("Remove").clicked() {
                    match self.store.remove(id) {
                        Ok(()) => close = true,
                        Err(e) => self.message(format!("Could not remove: {e:#}"), true),
                    }
                }
            });
        });
        ui.label(
            RichText::new(format!(
                "Saved as {}",
                self.store.dir().join(format!("{id}.txt")).display()
            ))
            .small()
            .weak(),
        );
        ui.separator();
        if entry.shots.is_empty() {
            ui.label(RichText::new("No screenshot recorded for this contract.").weak());
        }

        if self.preview.as_ref().is_none_or(|p| p.id != id) {
            self.preview = Some(Preview {
                id,
                images: entry
                    .shots
                    .iter()
                    .enumerate()
                    .map(|(i, s)| load_texture(ui.ctx(), s.source.as_deref(), &format!("{id}-{i}")))
                    .collect(),
            });
        }
        let images = self
            .preview
            .as_ref()
            .map(|p| p.images.as_slice())
            .unwrap_or_default();
        for (i, shot) in entry.shots.iter().enumerate() {
            let source = shot.source.as_deref();
            ui.label(source.map_or_else(|| "(unknown file)".to_owned(), file_name))
                .on_hover_text(source.map(|p| p.display().to_string()).unwrap_or_default());
            match images.get(i) {
                Some(Ok(texture)) => {
                    ui.add(egui::Image::new(texture).max_width(ui.available_width()));
                }
                Some(Err(e)) => {
                    ui.label(RichText::new(e).weak());
                }
                None => {}
            }
            egui::CollapsingHeader::new("OCR text")
                .id_salt(("ocr", id, i))
                .show(ui, |ui| ocr_text(ui, shot));
            ui.add_space(8.0);
        }
        close
    }
}

fn load_texture(
    ctx: &egui::Context,
    path: Option<&Path>,
    name: &str,
) -> Result<egui::TextureHandle, String> {
    let path = path.ok_or("No file recorded.")?;
    if !path.is_file() {
        return Err("File not found (moved or deleted).".into());
    }
    let img = sc_ocr::load_image(path).map_err(|e| e.to_string())?;
    let size = [img.width() as usize, img.height() as usize];
    let image = egui::ColorImage::from_rgb(size, img.as_raw());
    Ok(ctx.load_texture(name, image, egui::TextureOptions::LINEAR))
}

fn ocr_text(ui: &mut egui::Ui, shot: &ShotText) {
    for (label, text) in [
        ("Title", &shot.title),
        ("Reward", &shot.reward),
        ("Details", &shot.details),
        ("Objectives", &shot.objectives),
    ] {
        ui.label(RichText::new(label).weak());
        ui.label(RichText::new(text.as_str()).monospace().small());
    }
}

/// One contract in the column. The whole card is clickable.
fn card(ui: &mut egui::Ui, entry: &Entry, selected: bool) -> egui::Response {
    let c = &entry.contract;
    let visuals = ui.visuals();
    let stroke = if selected {
        Stroke::new(1.5_f32, visuals.selection.stroke.color)
    } else {
        Stroke::new(1.0_f32, visuals.widgets.noninteractive.bg_stroke.color)
    };
    egui::Frame::group(ui.style())
        .stroke(stroke)
        .fill(visuals.faint_bg_color)
        .inner_margin(10)
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(format::title(c)).strong());
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(
                        RichText::new(format::reward(c))
                            .strong()
                            .color(Color32::from_rgb(120, 200, 120)),
                    );
                });
            });
            let byline = format::byline(c);
            if !byline.is_empty() {
                ui.label(byline);
            }
            ui.label(RichText::new(format::facts(entry)).weak());
            if !c.cargo.is_empty() {
                ui.add_space(4.0);
                cargo_grid(ui, entry.id, c);
            }
        })
        .response
        .interact(Sense::click())
        .on_hover_cursor(egui::CursorIcon::PointingHand)
}

fn cargo_grid(ui: &mut egui::Ui, id: u64, c: &HaulingContract) {
    egui::Grid::new(("cargo", id))
        .num_columns(4)
        .spacing([14.0, 2.0])
        .striped(true)
        .show(ui, |ui| {
            for h in ["SCU", "Commodity", "Pick up", "Deliver to"] {
                ui.label(RichText::new(h).small().weak());
            }
            ui.end_row();
            for cargo in &c.cargo {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(RichText::new(cargo.scu.to_string()).monospace());
                });
                if cargo.restricted {
                    let text =
                        RichText::new(cargo.display_commodity()).color(ui.visuals().warn_fg_color);
                    ui.label(text).on_hover_text("Restricted cargo");
                } else {
                    ui.label(cargo.display_commodity());
                }
                ui.vertical(|ui| {
                    for p in &cargo.pickups {
                        place(ui, p);
                    }
                });
                place(ui, &cargo.dropoff);
                ui.end_row();
            }
        });
}

/// A place; inferred ones (not read on screen) get a `?` and the warning colour.
fn place(ui: &mut egui::Ui, p: &Place) {
    if p.uncertain {
        ui.label(RichText::new(p.to_string()).color(ui.visuals().warn_fg_color))
            .on_hover_text("Not on the screenshot: inferred from the other cargo lines");
    } else {
        ui.label(&p.name);
    }
}
