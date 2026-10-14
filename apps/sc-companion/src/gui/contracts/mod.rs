//! Contracts pane (a temporary view), ported from the hauling planner's contract board:
//! drop contract screenshots (OCR'd in the background) or paste `#HAULING v1` text, then
//! review each contract as a card, checking doubtful fields against the pixels they were
//! read from. Board order is oldest first (`#1` at the top).

mod card;
mod format;
mod widgets;
mod worker;

use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::Arc;

use eframe::egui::{self, Color32, Key, Modifiers, RichText, Sense, Stroke, TextEdit, Ui};
use sc_contracts::{Contract, Flow};
use sc_data::LoadedUniverse;
use sc_universe::normalize;
use widgets::{primary_button, small_button};
use worker::{Outcome, Worker};

use crate::contracts::board_text::{self, Issue};
use crate::contracts::fields::{self, Field};
use crate::contracts::places::{PlaceTree, Places};
use crate::contracts::reader::{EXTENSIONS, NO_OCR, file_name, is_screenshot};
use crate::contracts::review::{self, Lookup, Mark, Review};
use crate::contracts::store::Added;
use crate::contracts::{ContractReader, ContractStore, Entry, ShotText};
use crate::gui::theme;

/// Inserted by "Insert example".
const EXAMPLE: &str = "#HAULING v1

CONTRACT: Junior Rank - Small Cargo Haul
REWARD: 59,750 aUEC
CONTRACTOR: Covalex Independent Contractors
AVAILABILITY: 1h 4m
MAXBOX: 4
CARGO: 3 SCU | Aluminum (Ore) | Shubin Mining Facility SAL-5 on Lyria + Shubin Mining Facility SAL-2 on Lyria | Baijini Point above ArcCorp
CARGO: 4 SCU | Tungsten (Ore) | Shubin Mining Facility SAL-5 on Lyria | Baijini Point above ArcCorp
CARGO: 2 SCU | Corundum (Raw) | Shubin Mining Facility SAL-2 on Lyria | Baijini Point above ArcCorp
";

/// Widest the board gets, so cards stay readable on large windows.
const BOARD_MAX_W: f32 = 1100.0;

/// Screenshot textures, by file, loaded on first use.
#[derive(Default)]
pub struct Textures {
    map: HashMap<PathBuf, Option<egui::TextureHandle>>,
}

/// UI-only state of the board.
#[derive(Default)]
pub struct View {
    /// The field with keyboard focus (its source is shown under its card).
    pub focus: Option<(u64, Field)>,
    /// A field to focus on the next frame ("Next to check").
    pub goto: Option<(u64, Field)>,
    pub textures: Textures,
    /// Contracts whose whole screenshots are shown.
    pub open_shots: HashSet<u64>,
    /// Lines (contract, line) showing an empty slot for one more pickup, or drop-off.
    /// Only a view: the contract gets the place once one is typed.
    pub new_pickup: HashSet<(u64, usize)>,
    pub new_dropoff: HashSet<(u64, usize)>,
    pub confirm_remove: Option<u64>,
}

/// Changes collected while drawing, applied afterwards.
pub enum Edit {
    Set(u64, Field, String),
    RemoveContract(u64),
    AddLine(u64),
    RemoveLine(u64, usize),
    /// Use a suggested place for a misread one: (contract, written, suggestion).
    Fix(u64, String, String),
    Loaded(u64, usize, bool),
    ToggleShot(u64),
    Reparse(u64),
}

/// The review of an entry, kept until the entry changes.
pub struct Reviewed {
    entry: Entry,
    pub marks: HashMap<Field, Mark>,
    pub review: Review,
}

pub struct ContractsPane {
    data: Arc<LoadedUniverse>,
    store: ContractStore,
    /// `None` when this build has no OCR.
    worker: Option<Worker>,
    /// Last capture result, with its colour.
    status: (String, Color32),
    error: Option<String>,
    view: View,
    places: Places,
    /// Place name -> where it is (cleared when taught names change).
    trees: HashMap<String, PlaceTree>,
    reviews: HashMap<u64, Reviewed>,
    paste: String,
    /// Pasting replaces the list instead of adding to it.
    replace: bool,
    /// Lines of the last pasted text that couldn't be read.
    issues: Vec<Issue>,
    /// After the player replaced an unknown place name with a known place: (name as
    /// written, place), offered to be remembered.
    remember_offer: Option<(String, String)>,
    confirm_clear: bool,
}

impl ContractsPane {
    pub fn new(ctx: &egui::Context, data: Arc<LoadedUniverse>, dir: PathBuf) -> Self {
        let (store, warnings) = ContractStore::open(dir);
        let places = Places::new(&data.universe, store.place_names());
        let worker = ContractReader::available().then(|| Worker::start(ctx));
        let status = if worker.is_some() {
            (
                "Ready: drop contract screenshots below".to_owned(),
                theme::MUTED,
            )
        } else {
            (NO_OCR.to_owned(), theme::WARN)
        };
        Self {
            data,
            store,
            worker,
            status,
            error: (!warnings.is_empty())
                .then(|| format!("Skipped saved contracts:\n{}", warnings.join("\n"))),
            view: View::default(),
            places,
            trees: HashMap::new(),
            reviews: HashMap::new(),
            paste: String::new(),
            replace: false,
            issues: Vec::new(),
            remember_offer: None,
            confirm_clear: false,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        self.take_dropped_files(ctx);
        self.poll_worker();
        self.refresh_reviews();
        let next = ctx.input_mut(|i| i.consume_key(Modifiers::NONE, Key::F8));
        egui::CentralPanel::default()
            .frame(
                egui::Frame::new()
                    .fill(theme::BG)
                    .inner_margin(egui::Margin::symmetric(20, 10)),
            )
            .show(ctx, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("board-scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        ui.set_max_width(BOARD_MAX_W.min(ui.available_width()));
                        self.board(ui, next);
                    });
            });
    }

    // ---- input -----------------------------------------------------------------------

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
                self.status = (
                    format!(
                        "Skipped {}: not a PNG, JPEG or WebP image",
                        file_name(&path)
                    ),
                    theme::BAD,
                );
                continue;
            }
            match &mut self.worker {
                Some(worker) => worker.submit(path),
                None => {
                    self.status = (NO_OCR.to_owned(), theme::BAD);
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
                Outcome::Read(shot) => self.on_shot(shot),
                Outcome::Failed { path, error } => {
                    self.status = (
                        format!("Can't read {}: {error}", file_name(&path)),
                        theme::BAD,
                    );
                }
            }
        }
    }

    /// Adds a read screenshot and says what happened, as the planner's capture line does.
    fn on_shot(&mut self, shot: ShotText) {
        let name = shot.source.as_deref().map(file_name).unwrap_or_default();
        let added = match self.store.add_shot(shot, &self.data.universe) {
            Ok(added) => added,
            Err(e) => {
                self.status = (format!("{name}: could not save: {e:#}"), theme::BAD);
                return;
            }
        };
        let id = added.id();
        let Some(e) = self.store.get(id) else {
            return;
        };
        let c = &e.contract;
        let reward = c.reward.map_or_else(
            || "reward unread".to_owned(),
            |r| format!("{} aUEC", format::money(r)),
        );
        let lines = c.flows.len();
        let read = format!(
            "{lines} line{}, {} SCU, {reward}",
            if lines == 1 { "" } else { "s" },
            c.total_scu()
        );
        let n = self.number(id).unwrap_or(0);
        let place = match added {
            Added::New(_) => format!("added as #{n}"),
            Added::Merged(_) if e.stale => {
                format!("merged into #{n} (edited: re-parse to apply)")
            }
            Added::Merged(_) => format!("merged into #{n}"),
        };
        let to_check = review::to_check(std::slice::from_ref(e), self.lookup()).len();
        let mut line = format!("Read {name}: {read} \u{2014} {place}");
        if to_check > 0 {
            line.push_str(&format!(", {to_check} fields to check"));
        }
        let colour = if to_check > 0 || e.stale {
            theme::WARN
        } else {
            theme::GOOD
        };
        self.status = (line, colour);
    }

    // ---- review ----------------------------------------------------------------------

    fn lookup(&self) -> Lookup<'_> {
        Lookup {
            universe: &self.data.universe,
            places: &self.places,
        }
    }

    /// Recomputes the review of every entry that changed since it was last reviewed.
    fn refresh_reviews(&mut self) {
        let lk = Lookup {
            universe: &self.data.universe,
            places: &self.places,
        };
        let store = &self.store;
        self.reviews.retain(|id, _| store.get(*id).is_some());
        for e in store.entries() {
            if self.reviews.get(&e.id).is_some_and(|r| r.entry == *e) {
                continue;
            }
            let marks = fields::all(&e.contract)
                .into_iter()
                .map(|f| (f, review::mark(e, f, lk)))
                .collect();
            self.reviews.insert(
                e.id,
                Reviewed {
                    entry: e.clone(),
                    marks,
                    review: review::review(e, lk),
                },
            );
        }
    }

    /// Entries in board order: oldest first.
    fn board_entries(&self) -> Vec<&Entry> {
        self.store.entries().iter().rev().collect()
    }

    /// `#N` of an entry on the board.
    fn number(&self, id: u64) -> Option<usize> {
        self.board_entries()
            .iter()
            .position(|e| e.id == id)
            .map(|i| i + 1)
    }

    /// Every field that needs the player, in board then reading order.
    fn to_check(&self) -> Vec<(u64, Field)> {
        let mut out = Vec::new();
        for e in self.board_entries() {
            let Some(r) = self.reviews.get(&e.id) else {
                continue;
            };
            for f in fields::all(&e.contract) {
                if r.marks.get(&f).is_some_and(|m| *m != Mark::None) {
                    out.push((e.id, f));
                }
            }
        }
        out
    }

    /// Sort key of a field on the board: (contract position, field position).
    fn board_order(&self, x: (u64, Field)) -> Option<(usize, usize)> {
        let pos = self.board_entries().iter().position(|e| e.id == x.0)?;
        let e = self.store.get(x.0)?;
        let field = fields::all(&e.contract).iter().position(|f| *f == x.1)?;
        Some((pos, field))
    }

    // ---- drawing ---------------------------------------------------------------------

    fn board(&mut self, ui: &mut Ui, next_key: bool) {
        self.status_line(ui);
        self.drop_box(ui);
        self.paste_box(ui);
        self.error_box(ui);
        self.remember_box(ui);
        self.taught_names(ui);

        ui.add_space(4.0);
        let entries = self.board_entries();
        let scu: u32 = entries.iter().map(|e| e.contract.total_scu()).sum();
        let reward: u64 = entries.iter().filter_map(|e| e.contract.reward).sum();
        let count = entries.len();
        let saved_in = format!("Saved in {}", self.store.dir().display());
        ui.horizontal(|ui| {
            ui.heading(RichText::new("Contracts").size(15.0).strong())
                .on_hover_text(saved_in);
            ui.label(
                RichText::new(format!(
                    "{count} contracts \u{b7} {scu} SCU \u{b7} {} aUEC",
                    format::money(reward)
                ))
                .color(theme::MUTED)
                .size(13.0),
            );
        });
        let to_check = self.to_check();
        let mut go_next = next_key;
        ui.horizontal(|ui| {
            if small_button(ui, "Add contract").clicked() {
                let contract = Contract {
                    title: Some("New contract".into()),
                    flows: vec![Flow::default()],
                    ..Contract::default()
                };
                if let Err(e) = self.store.add_contract(contract) {
                    self.error = Some(format!("Couldn't save: {e:#}"));
                }
            }
            if small_button(ui, "Export as text").clicked() {
                let objectives: Vec<_> = self
                    .board_entries()
                    .iter()
                    .map(|e| e.contract.to_objectives())
                    .collect();
                self.paste = board_text::write(&objectives);
            }
            self.clear_button(ui, count);
            let label = format!("Next to check ({}) \u{b7} F8", to_check.len());
            let button = egui::Button::new(RichText::new(label).size(13.0).color(theme::WARN));
            if ui.add_enabled(!to_check.is_empty(), button).clicked() {
                go_next = true;
            }
        });
        if go_next {
            let focus = self.view.focus.and_then(|f| self.board_order(f));
            let after = |x: &&(u64, Field)| match (focus, self.board_order(**x)) {
                (Some(f), Some(o)) => o > f,
                _ => false,
            };
            self.view.goto = to_check.iter().find(after).or(to_check.first()).copied();
        }

        if count == 0 {
            egui::Frame::new()
                .stroke(Stroke::new(1.0_f32, theme::LINE))
                .corner_radius(theme::RADIUS)
                .inner_margin(16)
                .show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.label(
                        RichText::new(
                            "No contracts yet. Drop screenshots above, paste text, or add one \
                             by hand.",
                        )
                        .color(theme::MUTED),
                    );
                });
        }

        let duplicates = review::duplicates(self.store.entries());
        let mut edits = Vec::new();
        let mut focused = None;
        let ids: Vec<u64> = self.board_entries().iter().map(|e| e.id).collect();
        for (i, id) in ids.iter().enumerate() {
            let (Some(entry), Some(reviewed)) = (self.store.get(*id), self.reviews.get(id)) else {
                continue;
            };
            let dups: Vec<usize> = duplicates
                .iter()
                .filter_map(|&(a, b)| match (a == *id, b == *id) {
                    (true, _) => Some(b),
                    (_, true) => Some(a),
                    _ => None,
                })
                .filter_map(|d| ids.iter().position(|x| *x == d).map(|p| p + 1))
                .collect();
            let mut cx = card::Cx {
                entry,
                number: i + 1,
                reviewed,
                universe: &self.data.universe,
                places: &self.places,
                trees: &mut self.trees,
                view: &mut self.view,
                edits: &mut edits,
                focused: &mut focused,
                duplicates: dups,
            };
            card::show(ui, &mut cx);
            ui.add_space(6.0);
        }
        self.view.focus = focused;
        for e in edits {
            self.apply(e);
        }
    }

    fn clear_button(&mut self, ui: &mut Ui, count: usize) {
        if !self.confirm_clear {
            let button = egui::Button::new(RichText::new("Clear list").size(13.0));
            self.confirm_clear = ui.add_enabled(count > 0, button).clicked();
            return;
        }
        ui.label(
            RichText::new(format!("Remove all {count}?"))
                .color(theme::BAD)
                .size(13.0),
        );
        if small_button(ui, "Remove all").clicked() {
            self.confirm_clear = false;
            self.view = View {
                textures: std::mem::take(&mut self.view.textures),
                ..View::default()
            };
            if let Err(e) = self.store.clear() {
                self.error = Some(format!("Could not remove every contract: {e:#}"));
            }
        }
        if small_button(ui, "Cancel").clicked() {
            self.confirm_clear = false;
        }
    }

    fn status_line(&self, ui: &mut Ui) {
        let (text, color) = &self.status;
        ui.horizontal(|ui| {
            ui.label(RichText::new(theme::sym(ui.ctx(), "\u{25cf}", "*")).color(*color));
            ui.label(RichText::new(text).color(*color));
        });
    }

    fn drop_box(&mut self, ui: &mut Ui) {
        let Some(worker) = &self.worker else {
            return;
        };
        let hovering = ui.ctx().input(|i| !i.raw.hovered_files.is_empty());
        let (stroke, fill) = if hovering {
            (
                Stroke::new(2.0_f32, theme::ACCENT),
                theme::ACCENT.gamma_multiply(0.12),
            )
        } else {
            (Stroke::new(1.0_f32, theme::LINE), theme::PANEL)
        };
        let (pending, status) = (worker.pending(), worker.status().map(str::to_owned));
        let response = egui::Frame::new()
            .stroke(stroke)
            .fill(fill)
            .corner_radius(theme::RADIUS)
            .inner_margin(14)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                ui.vertical_centered(|ui| {
                    let text = if hovering {
                        "Release to read the screenshots"
                    } else {
                        "Drop contract screenshots here"
                    };
                    ui.label(RichText::new(text).strong().size(15.0));
                    ui.label(
                        RichText::new(
                            "or click to browse (PNG, JPEG, WebP; Contracts > Offers screen)",
                        )
                        .color(theme::MUTED)
                        .size(12.5),
                    );
                    if pending > 0 {
                        ui.horizontal(|ui| {
                            ui.spinner();
                            ui.label(status.as_deref().unwrap_or("Waiting..."));
                            if pending > 1 {
                                ui.label(
                                    RichText::new(format!("({} queued)", pending - 1))
                                        .color(theme::MUTED),
                                );
                            }
                        });
                    }
                });
            })
            .response
            .interact(Sense::click())
            .on_hover_cursor(egui::CursorIcon::PointingHand);
        if response.clicked()
            && let Some(paths) = rfd::FileDialog::new()
                .set_title("Contract screenshots")
                .add_filter("Screenshots", &EXTENSIONS)
                .pick_files()
        {
            self.submit(paths);
        }
        ui.add_space(4.0);
    }

    fn paste_box(&mut self, ui: &mut Ui) {
        theme::card().show(ui, |ui| {
            ui.add(
                TextEdit::multiline(&mut self.paste)
                    .desired_rows(4)
                    .desired_width(f32::INFINITY)
                    .font(egui::TextStyle::Monospace)
                    .hint_text(
                        "Paste #HAULING v1 text here, or type contracts in the same format.",
                    ),
            );
            ui.horizontal_wrapped(|ui| {
                if primary_button(ui, "Paste from clipboard", true).clicked() {
                    match arboard::Clipboard::new().and_then(|mut c| c.get_text()) {
                        Ok(text) => self.import(&text),
                        Err(_) => self.error = Some("The clipboard holds no text.".into()),
                    }
                }
                if ui
                    .add_enabled(
                        !self.paste.trim().is_empty(),
                        egui::Button::new("Read text box"),
                    )
                    .clicked()
                {
                    let text = std::mem::take(&mut self.paste);
                    self.import(&text);
                }
                ui.checkbox(
                    &mut self.replace,
                    RichText::new("Replace the list (otherwise add)")
                        .color(theme::MUTED)
                        .size(13.0),
                );
                if ui.link("Insert example").clicked() {
                    EXAMPLE.clone_into(&mut self.paste);
                }
            });
        });
        if !self.issues.is_empty() {
            egui::Frame::new()
                .stroke(Stroke::new(1.0_f32, theme::WARN))
                .corner_radius(theme::RADIUS)
                .inner_margin(8)
                .show(ui, |ui| {
                    for i in &self.issues {
                        let text = if i.line > 0 {
                            format!("Line {}: {}", i.line, i.message)
                        } else {
                            i.message.clone()
                        };
                        ui.label(RichText::new(text).color(theme::WARN).size(13.0));
                    }
                });
        }
    }

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
                ui.horizontal(|ui| {
                    ui.label(e);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.link("Dismiss").clicked() {
                            self.error = None;
                        }
                    });
                });
            });
    }

    fn remember_box(&mut self, ui: &mut Ui) {
        let Some((written, place)) = self.remember_offer.clone() else {
            return;
        };
        egui::Frame::new()
            .stroke(Stroke::new(1.0_f32, theme::ACCENT))
            .corner_radius(theme::RADIUS)
            .inner_margin(8)
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    ui.label(
                        RichText::new(format!(
                            "Remember \u{201c}{written}\u{201d} as {place} for later captures?"
                        ))
                        .size(13.0),
                    );
                    if small_button(ui, "Remember").clicked() {
                        self.remember_offer = None;
                        match self.store.remember(&written, &place) {
                            Ok(()) => self.places_changed(),
                            Err(e) => self.error = Some(format!("Couldn't save: {e:#}")),
                        }
                    }
                    if small_button(ui, "No").clicked() {
                        self.remember_offer = None;
                    }
                });
            });
    }

    /// The place names the player taught, each with a way to forget it.
    fn taught_names(&mut self, ui: &mut Ui) {
        let names = self.store.place_names().clone();
        if names.is_empty() {
            return;
        }
        let mut forget = None;
        egui::CollapsingHeader::new(
            RichText::new(format!("Place names you taught ({})", names.len()))
                .color(theme::MUTED)
                .size(13.0),
        )
        .id_salt("taught-names")
        .show(ui, |ui| {
            let arrow = theme::sym(ui.ctx(), "\u{2192}", "->");
            for (written, place) in &names {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{written} {arrow} {place}")).size(13.0));
                    if widgets::remove_button(ui, "Forget this name") {
                        forget = Some(written.clone());
                    }
                });
            }
        });
        if let Some(written) = forget {
            match self.store.forget(&written) {
                Ok(()) => self.places_changed(),
                Err(e) => self.error = Some(format!("Couldn't save: {e:#}")),
            }
        }
    }

    // ---- changes ---------------------------------------------------------------------

    fn import(&mut self, text: &str) {
        let (contracts, issues) = board_text::parse(text);
        self.issues = issues;
        if self.replace
            && let Err(e) = self.store.clear()
        {
            self.error = Some(format!("Could not clear the list: {e:#}"));
            return;
        }
        let n = contracts.len();
        for c in contracts {
            if let Err(e) = self.store.add_contract(Contract::from_objectives(&c)) {
                self.error = Some(format!("Couldn't save: {e:#}"));
                return;
            }
        }
        self.paste.clear();
        let colour = if self.issues.is_empty() {
            theme::GOOD
        } else {
            theme::WARN
        };
        self.status = (
            format!(
                "Added {n} contract{} from text",
                if n == 1 { "" } else { "s" }
            ),
            colour,
        );
    }

    /// Taught place names changed: rebuild the lookup and everything derived from it.
    fn places_changed(&mut self) {
        self.places = Places::new(&self.data.universe, self.store.place_names());
        self.trees.clear();
        self.reviews.clear();
    }

    fn tree_of(&self, name: &str) -> PlaceTree {
        self.places.tree(&self.data.universe, name)
    }

    /// After an unknown place name was replaced by a known place, offer to remember it.
    fn offer_to_remember(&mut self, written: &str, now: &str) {
        let known = matches!(self.tree_of(now), PlaceTree::Known { .. });
        if !written.trim().is_empty()
            && known
            && self.tree_of(written) == PlaceTree::Unknown
            && normalize(written) != normalize(now)
        {
            self.remember_offer = Some((written.trim().to_owned(), now.trim().to_owned()));
        }
    }

    fn apply(&mut self, edit: Edit) {
        let result = match edit {
            Edit::Set(id, f, value) => {
                let before = self
                    .store
                    .get(id)
                    .map(|e| fields::text(&e.contract, f))
                    .unwrap_or_default();
                let r = self.store.edit(id, f, &value);
                if r.is_ok() && f.is_place() {
                    self.offer_to_remember(&before, &value);
                }
                r
            }
            Edit::RemoveContract(id) => {
                self.view.confirm_remove = None;
                self.view.open_shots.remove(&id);
                self.store.remove(id)
            }
            Edit::AddLine(id) => self.store.add_line(id),
            Edit::RemoveLine(id, k) => {
                self.view.new_pickup.retain(|x| x.0 != id);
                self.view.new_dropoff.retain(|x| x.0 != id);
                self.store.remove_line(id, k)
            }
            Edit::Fix(id, written, suggestion) => {
                let r = self.store.apply_fix(id, &written, &suggestion);
                if r.is_ok() {
                    self.offer_to_remember(&written, &suggestion);
                }
                r
            }
            Edit::Loaded(id, k, on) => self.store.set_loaded(id, k, on),
            Edit::ToggleShot(id) => {
                if !self.view.open_shots.remove(&id) {
                    self.view.open_shots.insert(id);
                }
                Ok(())
            }
            Edit::Reparse(id) => self.store.reparse(id, &self.data.universe),
        };
        if let Err(e) = result {
            self.error = Some(format!("Couldn't save the contract: {e:#}"));
        }
    }
}
