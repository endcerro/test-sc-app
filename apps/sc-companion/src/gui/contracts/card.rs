//! One contract as an editable card: header fields, one row per flow (SCU | cargo | pickups
//! | drop-offs | loaded), checks, the capture and a footer. Painting and editing only; the
//! pane applies the returned actions.

use std::collections::HashMap;
use std::path::Path;

use eframe::egui::{self, Button, Color32, RichText, Sense, Stroke, TextEdit, vec2};
use sc_contracts::snap::resolve_place;
use sc_universe::Universe;

use super::draft::{Draft, FlowDraft, PlaceDraft};
use super::format;
use crate::contracts::checks::checks;
use crate::contracts::reader::file_name;
use crate::contracts::{Entry, ShotText};

const REWARD_COLOR: Color32 = Color32::from_rgb(230, 190, 90);
const SCU_COLOR: Color32 = Color32::from_rgb(175, 165, 255);
const RESOLVED_COLOR: Color32 = Color32::from_rgb(120, 190, 130);

/// What the player did on a card.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Save the draft.
    Commit,
    AskRemove,
    CancelRemove,
    Remove,
    /// Rebuild from the screenshots, discarding edits.
    Reparse,
}

/// Place name -> universe path (`Stanton › Crusader › Yela`), memoised across frames.
pub struct Resolver<'a> {
    pub universe: &'a Universe,
    pub cache: &'a mut HashMap<String, Option<String>>,
}

impl Resolver<'_> {
    pub fn path(&mut self, name: &str) -> Option<String> {
        let name = name.trim();
        if name.is_empty() {
            return None;
        }
        let u = self.universe;
        self.cache
            .entry(name.to_owned())
            .or_insert_with(|| resolve_place(u, name).map(|m| u.path_string(m.id)))
            .clone()
    }
}

/// Screenshot textures per entry, loaded when "Show capture" is first opened.
pub type Captures = HashMap<u64, Vec<Result<egui::TextureHandle, String>>>;

pub struct Card<'a, 'r> {
    /// `#N`, by age (oldest is 1).
    pub number: usize,
    pub entry: &'a Entry,
    pub draft: &'a mut Draft,
    pub resolver: &'a mut Resolver<'r>,
    pub captures: &'a mut Captures,
    pub confirm_remove: bool,
}

impl Card<'_, '_> {
    pub fn show(self, ui: &mut egui::Ui) -> Vec<Action> {
        let Self {
            number,
            entry,
            draft,
            resolver,
            captures,
            confirm_remove,
        } = self;
        let mut actions = Vec::new();
        let mut commit = false;
        let visuals = ui.visuals().clone();
        egui::Frame::group(ui.style())
            .fill(visuals.faint_bg_color)
            .inner_margin(12)
            .show(ui, |ui| {
                ui.set_min_width(ui.available_width());
                title_row(ui, number, draft, confirm_remove, &mut commit, &mut actions);
                ui.add_space(4.0);
                commit |= header_fields(ui, draft);
                ui.add_space(8.0);
                commit |= flow_grid(ui, entry.id, draft, resolver);
                ui.add_space(4.0);
                if ui.button("Add cargo line").clicked() {
                    draft.flows.push(FlowDraft::empty());
                    commit = true;
                }
                ui.add_space(4.0);
                capture(ui, entry, captures, &mut actions);
                let mut known = |name: &str| resolver.path(name).is_some();
                for check in checks(entry, &mut known) {
                    ui.label(RichText::new(format!("Check: {check}")).color(visuals.warn_fg_color));
                }
                ui.label(RichText::new(format::footer(entry)).small().weak());
            });
        if commit {
            actions.push(Action::Commit);
        }
        actions
    }
}

fn title_row(
    ui: &mut egui::Ui,
    number: usize,
    draft: &mut Draft,
    confirm_remove: bool,
    commit: &mut bool,
    actions: &mut Vec<Action>,
) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("#{number}")).weak());
        let reserve = if confirm_remove { 190.0 } else { 30.0 };
        let width = (ui.available_width() - reserve).max(200.0);
        let title = TextEdit::singleline(&mut draft.title)
            .desired_width(width)
            .hint_text("Title");
        *commit |= ui.add(title).lost_focus();
        if confirm_remove {
            ui.label("Remove this contract?");
            if ui
                .button(RichText::new("Remove").color(ui.visuals().error_fg_color))
                .clicked()
            {
                actions.push(Action::Remove);
            }
            if ui.button("Keep").clicked() {
                actions.push(Action::CancelRemove);
            }
        } else if ui
            .small_button("x")
            .on_hover_text("Remove this contract")
            .clicked()
        {
            actions.push(Action::AskRemove);
        }
    });
}

/// Labelled header fields. Returns true when one was committed.
fn header_fields(ui: &mut egui::Ui, draft: &mut Draft) -> bool {
    let mut commit = false;
    ui.horizontal_wrapped(|ui| {
        if field(
            ui,
            "REWARD (AUEC)",
            &mut draft.reward,
            110.0,
            Some(REWARD_COLOR),
        ) {
            draft.reward = format::reward(format::parse_number(&draft.reward));
            commit = true;
        }
        commit |= field(ui, "CONTRACTOR", &mut draft.contractor, 220.0, None);
        commit |= field(ui, "AVAILABILITY", &mut draft.availability, 90.0, None);
        commit |= field(ui, "LARGEST BOX (SCU)", &mut draft.max_box, 80.0, None);
        commit |= field(ui, "REP", &mut draft.rep, 60.0, None);
    });
    commit
}

fn field(
    ui: &mut egui::Ui,
    label: &str,
    text: &mut String,
    width: f32,
    color: Option<Color32>,
) -> bool {
    ui.vertical(|ui| {
        ui.label(RichText::new(label).small().weak());
        let mut edit = TextEdit::singleline(text).desired_width(width);
        if let Some(c) = color {
            edit = edit.text_color(c);
        }
        ui.add(edit).lost_focus()
    })
    .inner
}

/// The flow rows. Returns true when something was committed.
fn flow_grid(ui: &mut egui::Ui, id: u64, draft: &mut Draft, resolver: &mut Resolver) -> bool {
    let mut commit = false;
    let mut remove = None;
    // SCU, cargo, loaded and remove columns are fixed; the place columns share the rest.
    let fixed = 50.0 + 210.0 + 50.0 + 20.0 + 5.0 * 12.0;
    let place_width = ((ui.available_width() - fixed) / 2.0 - 60.0).clamp(150.0, 340.0);
    egui::Grid::new(("flows", id))
        .num_columns(6)
        .spacing([12.0, 6.0])
        .show(ui, |ui| {
            for h in ["SCU", "CARGO", "PICKUPS", "DROP-OFFS", "LOADED", ""] {
                ui.label(RichText::new(h).small().weak());
            }
            ui.end_row();
            for (i, flow) in draft.flows.iter_mut().enumerate() {
                commit |= flow_row(ui, flow, place_width, resolver);
                if ui.checkbox(&mut flow.loaded, "").changed() {
                    commit = true;
                }
                if ui
                    .small_button("x")
                    .on_hover_text("Remove this line")
                    .clicked()
                {
                    remove = Some(i);
                }
                ui.end_row();
            }
        });
    if let Some(i) = remove {
        draft.flows.remove(i);
        commit = true;
    }
    commit
}

/// SCU, cargo, pickups and drop-offs cells of one flow.
fn flow_row(
    ui: &mut egui::Ui,
    flow: &mut FlowDraft,
    place_width: f32,
    resolver: &mut Resolver,
) -> bool {
    let mut commit = false;
    // One drop-off: its amount is the line's SCU. Several: the total, each drop-off has its
    // own amount field.
    if let [drop] = flow.dropoffs.as_mut_slice() {
        let edit = TextEdit::singleline(&mut drop.scu)
            .desired_width(40.0)
            .text_color(SCU_COLOR);
        commit |= ui.add(edit).lost_focus();
    } else {
        ui.label(RichText::new(flow.scu().to_string()).color(SCU_COLOR))
            .on_hover_text("Total of the drop-offs");
    }

    ui.horizontal(|ui| {
        let toggle = Button::selectable(flow.restricted, "[!]");
        if ui.add(toggle).on_hover_text("Restricted cargo").clicked() {
            flow.restricted = !flow.restricted;
            commit = true;
        }
        let edit = TextEdit::singleline(&mut flow.commodity)
            .desired_width(160.0)
            .hint_text("Commodity");
        commit |= ui.add(edit).lost_focus();
    });

    let (can_add_pickup, can_add_dropoff) = (flow.can_add_pickup(), flow.can_add_dropoff());
    let several_drops = flow.dropoffs.len() > 1;
    ui.vertical(|ui| {
        commit |= place_list(
            ui,
            &mut flow.pickups,
            can_add_pickup,
            false,
            place_width,
            resolver,
        );
    });
    ui.vertical(|ui| {
        commit |= place_list(
            ui,
            &mut flow.dropoffs,
            can_add_dropoff,
            several_drops,
            place_width,
            resolver,
        );
    });
    commit
}

/// A list of places, each with its resolved path below, `x` to remove (when there are
/// several) and `+` after the last one.
fn place_list(
    ui: &mut egui::Ui,
    places: &mut Vec<PlaceDraft>,
    can_add: bool,
    with_scu: bool,
    width: f32,
    resolver: &mut Resolver,
) -> bool {
    let mut commit = false;
    let mut remove = None;
    let mut add = false;
    let count = places.len();
    for (i, place) in places.iter_mut().enumerate() {
        ui.horizontal(|ui| {
            let mut edit = TextEdit::singleline(&mut place.name).desired_width(width);
            if place.uncertain {
                edit = edit.text_color(ui.visuals().warn_fg_color);
            }
            let response = ui.add(edit);
            if response.changed() {
                place.uncertain = false;
            }
            commit |= response.lost_focus();
            if with_scu {
                let scu = TextEdit::singleline(&mut place.scu)
                    .desired_width(36.0)
                    .text_color(SCU_COLOR);
                commit |= ui.add(scu).on_hover_text("SCU").lost_focus();
            }
            if count > 1 && ui.small_button("x").on_hover_text("Remove").clicked() {
                remove = Some(i);
            }
            if i + 1 == count {
                add |= add_button(ui, can_add);
            }
        });
        place_status(ui, place, resolver);
    }
    if count == 0 {
        add |= add_button(ui, can_add);
    }
    if let Some(i) = remove {
        places.remove(i);
        commit = true;
    }
    if add {
        places.push(PlaceDraft::default());
        commit = true;
    }
    commit
}

fn add_button(ui: &mut egui::Ui, enabled: bool) -> bool {
    ui.add_enabled(enabled, Button::new("+").small())
        .on_hover_text("Add a place")
        .on_disabled_hover_text(
            "A line has one pickup or one drop-off: remove the extra places on the other side \
             first",
        )
        .clicked()
}

/// Under a place: its universe path with a check mark, or why it is doubtful.
fn place_status(ui: &mut egui::Ui, place: &PlaceDraft, resolver: &mut Resolver) {
    if place.name.trim().is_empty() {
        return;
    }
    let warn = ui.visuals().warn_fg_color;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        match resolver.path(&place.name) {
            Some(path) => {
                check_mark(ui, RESOLVED_COLOR);
                ui.label(RichText::new(path).small().color(RESOLVED_COLOR));
            }
            None => {
                ui.label(RichText::new("not in universe data").small().color(warn));
            }
        }
        if place.uncertain {
            ui.label(RichText::new("(inferred)").small().color(warn))
                .on_hover_text("Its Collect line was not on the screenshot");
        }
    });
}

/// A small painted tick (egui's default fonts have no check mark glyph).
fn check_mark(ui: &mut egui::Ui, color: Color32) {
    let (rect, _) = ui.allocate_exact_size(vec2(9.0, 9.0), Sense::hover());
    let stroke = Stroke::new(1.5_f32, color);
    let bottom = rect.center_bottom() + vec2(-1.0, -1.0);
    ui.painter()
        .line_segment([rect.left_center() + vec2(0.0, 0.5), bottom], stroke);
    ui.painter()
        .line_segment([bottom, rect.right_top()], stroke);
}

/// "Show capture": screenshots, their OCR text and re-parsing.
fn capture(ui: &mut egui::Ui, entry: &Entry, captures: &mut Captures, actions: &mut Vec<Action>) {
    egui::CollapsingHeader::new("Show capture")
        .id_salt(("capture", entry.id))
        .show(ui, |ui| {
            if entry.shots.is_empty() {
                ui.label(RichText::new("No screenshot recorded for this contract.").weak());
                return;
            }
            let images = captures.entry(entry.id).or_default();
            if images.len() != entry.shots.len() {
                *images = entry
                    .shots
                    .iter()
                    .enumerate()
                    .map(|(i, s)| {
                        let name = format!("capture-{}-{i}", entry.id);
                        load_texture(ui.ctx(), s.source.as_deref(), &name)
                    })
                    .collect();
            }
            for (i, shot) in entry.shots.iter().enumerate() {
                let source = shot.source.as_deref();
                ui.label(source.map_or_else(|| "(unknown file)".to_owned(), file_name))
                    .on_hover_text(source.map(|p| p.display().to_string()).unwrap_or_default());
                match images.get(i) {
                    Some(Ok(texture)) => {
                        let width = ui.available_width().min(960.0);
                        ui.add(egui::Image::new(texture).max_width(width));
                    }
                    Some(Err(e)) => {
                        ui.label(RichText::new(e).weak());
                    }
                    None => {}
                }
                egui::CollapsingHeader::new("OCR text")
                    .id_salt(("ocr", entry.id, i))
                    .show(ui, |ui| ocr_text(ui, shot));
            }
            let label = if entry.edited {
                "Re-parse from screenshots (discards edits)"
            } else {
                "Re-parse from screenshots"
            };
            if ui.button(label).clicked() {
                actions.push(Action::Reparse);
            }
        });
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
