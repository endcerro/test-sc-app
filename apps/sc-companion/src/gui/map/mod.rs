//! Map pane: a 2D top-down / 3D tilted view of one star system, with focus drill-down
//! (system → planet → moon), search, layer toggles and an inspector.

mod camera;
mod inspector;
mod paint;
mod scene;
mod style;

use std::sync::Arc;

use camera::{Camera, ViewMode};
use eframe::egui::{self, Color32, CursorIcon, Key, PointerButton, RichText, Sense};
use paint::MapPainter;
use sc_data::LoadedUniverse;
use sc_universe::{LocationId, LocationKind, LookupOptions, Universe};
use scene::{Filters, MapIndex, SceneBuilder, anchor};

const BACKGROUND: Color32 = Color32::from_rgb(10, 12, 18);
/// Smallest framing extent, so leaf locations (a city) still show some surroundings.
const MIN_FRAME_EXTENT: f64 = 20_000.0;

/// Something the UI asked for; applied after the frame's widgets are drawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Select(Option<LocationId>),
    Focus(LocationId),
    MeasureFrom(Option<LocationId>),
}

pub struct MapPane {
    data: Arc<LoadedUniverse>,
    index: MapIndex,
    camera: Camera,
    filters: Filters,
    system: LocationId,
    focus: LocationId,
    selected: Option<LocationId>,
    measure_from: Option<LocationId>,
    search: String,
    highlight_trading: bool,
    labels: bool,
    /// The first frame frames the focus without animating, once the viewport is known.
    framed: bool,
}

impl MapPane {
    /// `data` must contain at least one system.
    pub fn new(data: Arc<LoadedUniverse>) -> Self {
        let u = &data.universe;
        let system = u
            .system("Stanton")
            .or_else(|| u.systems().next())
            .map_or_else(|| u.locations()[0].id, |s| s.id);
        Self {
            index: MapIndex::new(u),
            camera: Camera::default(),
            filters: Filters::default(),
            system,
            focus: system,
            selected: None,
            measure_from: None,
            search: String::new(),
            highlight_trading: true,
            labels: true,
            framed: false,
            data,
        }
    }

    pub fn show(&mut self, ctx: &egui::Context) {
        let data = self.data.clone();
        let u = &data.universe;
        let mut actions = Vec::new();

        egui::SidePanel::left("map_controls")
            .default_width(270.0)
            .show(ctx, |ui| {
                egui::ScrollArea::vertical().show(ui, |ui| self.controls(ui, u, &mut actions));
            });
        if let Some(selected) = self.selected {
            egui::SidePanel::right("map_inspector")
                .default_width(320.0)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical().show(ui, |ui| {
                        inspector::show(
                            ui,
                            u,
                            selected,
                            self.focus,
                            self.measure_from,
                            &mut actions,
                        );
                    });
                });
        }
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(BACKGROUND))
            .show(ctx, |ui| {
                self.breadcrumbs(ui, u, &mut actions);
                self.canvas(ui, u, &mut actions);
            });

        for action in actions {
            self.apply(u, action);
        }
    }

    fn apply(&mut self, u: &Universe, action: Action) {
        match action {
            Action::Select(id) => self.selected = id,
            Action::MeasureFrom(id) => self.measure_from = id,
            Action::Focus(id) => {
                let l = u.get(id);
                self.system = l.system;
                self.focus = id;
                let extent = self
                    .index
                    .cluster_radius(id)
                    .max(l.radius_m.unwrap_or(0.0) * 1.5)
                    .max(MIN_FRAME_EXTENT);
                let center = anchor(u, id).unwrap_or_default();
                self.camera.frame(center, extent);
            }
        }
    }

    /// The location one level up from `id` for drill-down (stars are skipped: the system
    /// view is centred on the star already).
    fn parent_focus(u: &Universe, id: LocationId) -> LocationId {
        match u.parent(id) {
            Some(p) if p.kind == LocationKind::Star => p.parent.unwrap_or(p.id),
            Some(p) => p.id,
            None => id,
        }
    }

    fn controls(&mut self, ui: &mut egui::Ui, u: &Universe, actions: &mut Vec<Action>) {
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            ui.label("System");
            egui::ComboBox::from_id_salt("map_system")
                .selected_text(&u.get(self.system).name)
                .show_ui(ui, |ui| {
                    for sys in u.systems() {
                        if ui
                            .selectable_label(sys.id == self.system, &sys.name)
                            .clicked()
                        {
                            actions.push(Action::Focus(sys.id));
                        }
                    }
                });
        });

        ui.add_space(6.0);
        ui.add(
            egui::TextEdit::singleline(&mut self.search)
                .hint_text("Search location or venue…")
                .desired_width(f32::INFINITY),
        );
        self.search_results(ui, u, actions);

        ui.separator();
        ui.horizontal(|ui| {
            ui.label("View");
            ui.selectable_value(&mut self.camera.mode, ViewMode::TopDown, "2D top-down");
            ui.selectable_value(&mut self.camera.mode, ViewMode::Tilted, "3D");
        });
        ui.checkbox(&mut self.filters.orbits, "Orbits");
        ui.checkbox(&mut self.labels, "Labels");
        ui.checkbox(&mut self.highlight_trading, "Highlight trading locations")
            .on_hover_text("Green ring around locations with a commodity terminal");
        ui.checkbox(&mut self.filters.show_hidden, "Hidden on starmap (dimmed)");
        ui.checkbox(
            &mut self.filters.show_inherited,
            "Approximate positions (hollow)",
        )
        .on_hover_text("No coordinates in the data: drawn at their parent's position");

        ui.separator();
        egui::CollapsingHeader::new("Layers")
            .default_open(true)
            .show(ui, |ui| {
                for kind in style::LAYERS {
                    let mut on = !self.filters.hidden_kinds.contains(&kind);
                    ui.horizontal(|ui| {
                        let (rect, _) =
                            ui.allocate_exact_size(egui::vec2(12.0, 12.0), Sense::hover());
                        paint::swatch(ui.painter(), rect.center(), kind);
                        if ui.checkbox(&mut on, kind.label()).changed() {
                            if on {
                                self.filters.hidden_kinds.remove(&kind);
                            } else {
                                self.filters.hidden_kinds.insert(kind);
                            }
                        }
                    });
                }
            });

        egui::CollapsingHeader::new("Controls")
            .default_open(false)
            .show(ui, |ui| {
                for line in [
                    "Scroll: zoom at cursor",
                    "Drag: pan (2D) / orbit (3D)",
                    "Right / middle / shift-drag: pan",
                    "Click: select · double-click: focus",
                    "Backspace / Esc: focus up one level",
                    "F: focus selection · Home: whole system",
                ] {
                    ui.label(RichText::new(line).small());
                }
            });
    }

    fn search_results(&mut self, ui: &mut egui::Ui, u: &Universe, actions: &mut Vec<Action>) {
        let query = self.search.trim();
        if query.chars().count() < 2 {
            return;
        }
        let matches = u.find_fuzzy(
            query,
            LookupOptions {
                scope: None,
                limit: 10,
                min_score: 0.6,
            },
        );
        if matches.is_empty() {
            ui.label(RichText::new("No match").weak());
        }
        for m in matches {
            let l = u.get(m.id);
            let mut text = format!("{}  ·  {}", l.name, l.kind.label());
            if let Some(alias) = u.matched_alias(&m) {
                text.push_str(&format!("\nvia {} \"{}\"", alias.kind.label(), alias.name));
            }
            let resp = ui
                .selectable_label(self.selected == Some(m.id), text)
                .on_hover_text(u.path_string(m.id));
            if resp.clicked() {
                actions.push(Action::Select(Some(m.id)));
                actions.push(Action::Focus(m.id));
            }
        }
    }

    fn breadcrumbs(&self, ui: &mut egui::Ui, u: &Universe, actions: &mut Vec<Action>) {
        let mut chain: Vec<LocationId> = u
            .ancestors(self.focus)
            .filter(|a| a.kind != LocationKind::Star)
            .map(|a| a.id)
            .collect();
        chain.reverse();
        chain.push(self.focus);
        egui::Frame::NONE
            .inner_margin(egui::Margin::symmetric(8, 4))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    for (i, id) in chain.iter().enumerate() {
                        if i > 0 {
                            ui.label(RichText::new("›").weak());
                        }
                        let name = &u.get(*id).name;
                        let current = *id == self.focus;
                        if ui.selectable_label(current, name).clicked() && !current {
                            actions.push(Action::Focus(*id));
                        }
                    }
                });
            });
    }

    fn canvas(&mut self, ui: &mut egui::Ui, u: &Universe, actions: &mut Vec<Action>) {
        let (response, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
        self.camera.viewport = response.rect;
        if !self.framed {
            self.framed = true;
            let extent = self.index.cluster_radius(self.focus).max(MIN_FRAME_EXTENT);
            self.camera
                .frame_now(anchor(u, self.focus).unwrap_or_default(), extent);
        }

        // Camera input.
        let hover_pos = response.hover_pos();
        if let Some(p) = hover_pos {
            let (scroll, zoom) = ui.input(|i| (i.smooth_scroll_delta.y, i.zoom_delta()));
            let factor = f64::from(zoom) * (f64::from(scroll) * 0.003).exp();
            if (factor - 1.0).abs() > 1e-6 {
                self.camera.zoom_at(p, factor);
            }
        }
        let shift = ui.input(|i| i.modifiers.shift);
        let delta = response.drag_delta();
        if response.dragged_by(PointerButton::Primary) {
            if self.camera.mode == ViewMode::Tilted && !shift {
                self.camera.orbit(delta);
            } else {
                self.camera.pan(delta);
            }
        } else if response.dragged_by(PointerButton::Secondary)
            || response.dragged_by(PointerButton::Middle)
        {
            self.camera.pan(delta);
        }
        if !ui.ctx().wants_keyboard_input() {
            ui.input(|i| {
                if i.key_pressed(Key::Backspace) || i.key_pressed(Key::Escape) {
                    actions.push(Action::Focus(Self::parent_focus(u, self.focus)));
                }
                if i.key_pressed(Key::F)
                    && let Some(sel) = self.selected
                {
                    actions.push(Action::Focus(sel));
                }
                if i.key_pressed(Key::Home) {
                    actions.push(Action::Focus(self.system));
                }
            });
        }
        let dt = ui.input(|i| i.stable_dt).min(0.1);
        if self.camera.animate(f64::from(dt)) {
            ui.ctx().request_repaint();
        }

        // Draw.
        let scene = SceneBuilder {
            universe: u,
            index: &self.index,
            camera: &self.camera,
            filters: &self.filters,
            system: self.system,
            focus: self.focus,
        }
        .build();
        let hovered = hover_pos.and_then(|p| scene.pick(p)).map(|it| it.id);
        let jump_labels = MapPainter {
            painter: &painter,
            universe: u,
            scene: &scene,
            camera: &self.camera,
            focus: self.focus,
            selected: self.selected,
            hovered,
            highlight_trading: self.highlight_trading,
            labels: self.labels,
        }
        .paint();
        let jump_hit = hover_pos.and_then(|p| {
            jump_labels
                .iter()
                .find(|(rect, _)| rect.contains(p))
                .map(|(_, to)| *to)
        });

        // Selection.
        if response.clicked() {
            match jump_hit {
                Some(to) => {
                    actions.push(Action::Select(Some(to)));
                    actions.push(Action::Focus(to));
                }
                None => actions.push(Action::Select(hovered)),
            }
        }
        if response.double_clicked()
            && let Some(id) = hovered
        {
            actions.push(Action::Focus(id));
        }
        if hovered.is_some() || jump_hit.is_some() {
            ui.ctx().set_cursor_icon(CursorIcon::PointingHand);
        }
        if let Some(id) = hovered
            && !response.dragged()
        {
            response.on_hover_ui_at_pointer(|ui| self.tooltip(ui, u, id));
        }
    }

    fn tooltip(&self, ui: &mut egui::Ui, u: &Universe, id: LocationId) {
        let l = u.get(id);
        ui.strong(&l.name);
        ui.label(RichText::new(u.path_string(id)).weak());
        let mut facts = vec![l.kind.label().to_owned()];
        if l.has_commodity_trading() {
            facts.push("trading".into());
        }
        if l.hidden {
            facts.push("hidden on starmap".into());
        }
        if !l.aliases.is_empty() {
            facts.push(format!("{} aliases", l.aliases.len()));
        }
        ui.label(facts.join(" · "));
        let origin = self.measure_from.unwrap_or(self.focus);
        if origin != id
            && u.get(origin).kind != LocationKind::System
            && let Ok(d) = u.distance(origin, id)
        {
            ui.label(format!("{d} from {}", u.get(origin).name));
        }
    }
}
