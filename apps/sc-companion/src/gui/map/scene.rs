//! What to draw for the current camera: pure data, no painting (so it can be tested).

use std::collections::HashSet;

use eframe::egui::{Pos2, Rect};
use sc_universe::{Location, LocationId, LocationKind, Position, Universe, Vec3};

use super::camera::{Camera, ViewMode, nice_length};
use super::style::kind_style;

/// A location's children are drawn once its subtree spans this many pixels.
const EXPAND_PX: f32 = 30.0;
/// Hover/click tolerance around a marker, in pixels.
const PICK_SLOP: f32 = 6.0;

/// What the user chose to show.
#[derive(Debug, Clone)]
pub struct Filters {
    pub hidden_kinds: HashSet<LocationKind>,
    /// Show locations hidden on the in-game starmap (drawn dimmed).
    pub show_hidden: bool,
    /// Show locations whose position is copied from an ancestor (drawn hollow).
    pub show_inherited: bool,
    pub orbits: bool,
}

impl Default for Filters {
    fn default() -> Self {
        Self {
            hidden_kinds: HashSet::from([LocationKind::Asteroid]),
            show_hidden: true,
            show_inherited: true,
            orbits: true,
        }
    }
}

impl Filters {
    pub fn shows(&self, l: &Location) -> bool {
        !self.hidden_kinds.contains(&l.kind)
            && (self.show_hidden || !l.hidden)
            && (self.show_inherited || !matches!(l.position, Position::Inherited { .. }))
    }
}

/// Per-universe data precomputed once for the map.
#[derive(Debug)]
pub struct MapIndex {
    /// Radius of the sphere around each location that contains its whole subtree, metres.
    cluster: Vec<f64>,
}

impl MapIndex {
    pub fn new(u: &Universe) -> Self {
        let mut cluster: Vec<f64> = u
            .locations()
            .iter()
            .map(|l| l.radius_m.unwrap_or(0.0))
            .collect();
        for l in u.locations() {
            let Some(pos) = anchor(u, l.id) else {
                continue;
            };
            let reach = l.radius_m.unwrap_or(0.0);
            for a in u.ancestors(l.id) {
                if let Some(apos) = anchor(u, a.id) {
                    let r = &mut cluster[a.id.index()];
                    *r = r.max(apos.distance_to(pos) + reach);
                }
            }
        }
        Self { cluster }
    }

    /// Radius of the sphere containing the location and everything below it, metres.
    pub fn cluster_radius(&self, id: LocationId) -> f64 {
        self.cluster.get(id.index()).copied().unwrap_or(0.0)
    }
}

/// Where a location is drawn. System roots sit on their star (the origin).
pub fn anchor(u: &Universe, id: LocationId) -> Option<Vec3> {
    let l = u.get(id);
    match l.kind {
        LocationKind::System => Some(Vec3::ZERO),
        _ => l.position.vec(),
    }
}

/// Centre of the orbit drawn for planets (the star) and moons (their parent body).
fn orbit_center(u: &Universe, l: &Location) -> Option<Vec3> {
    match l.kind {
        // Planets always orbit the star, even when the data parents one to another planet.
        LocationKind::Planet => Some(Vec3::ZERO),
        LocationKind::Moon => u
            .ancestors(l.id)
            .find(|a| a.kind.is_body())
            .and_then(|a| anchor(u, a.id)),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SceneItem {
    pub id: LocationId,
    pub pos: Pos2,
    pub radius: f32,
    pub depth: f64,
    /// Number of shown descendants folded into this marker (0 when expanded).
    pub collapsed: usize,
}

/// A jump point end on screen, linking to the other side.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct JumpMarker {
    pub pos: Pos2,
    pub at: LocationId,
    pub to: LocationId,
}

#[derive(Debug, Default)]
pub struct Scene {
    /// Back to front.
    pub items: Vec<SceneItem>,
    pub orbits: Vec<Vec<Pos2>>,
    pub grid: Vec<Vec<Pos2>>,
    pub drop_lines: Vec<[Pos2; 2]>,
    pub jumps: Vec<JumpMarker>,
}

impl Scene {
    /// The item under `p`: the one whose centre is closest among those within reach.
    pub fn pick(&self, p: Pos2) -> Option<&SceneItem> {
        self.items
            .iter()
            .rev()
            .map(|it| (it, it.pos.distance(p)))
            .filter(|(it, d)| *d <= it.radius.max(3.0) + PICK_SLOP)
            .min_by(|a, b| a.1.total_cmp(&b.1))
            .map(|(it, _)| it)
    }

    #[cfg(test)]
    pub fn item(&self, id: LocationId) -> Option<&SceneItem> {
        self.items.iter().find(|it| it.id == id)
    }
}

pub struct SceneBuilder<'a> {
    pub universe: &'a Universe,
    pub index: &'a MapIndex,
    pub camera: &'a Camera,
    pub filters: &'a Filters,
    pub system: LocationId,
    pub focus: LocationId,
}

impl SceneBuilder<'_> {
    pub fn build(&self) -> Scene {
        let mut scene = Scene::default();
        if self.camera.mode == ViewMode::Tilted {
            self.grid(&mut scene);
        }
        self.visit(self.system, &mut scene);
        scene.items.sort_by(|a, b| a.depth.total_cmp(&b.depth));
        self.jumps(&mut scene);
        scene
    }

    fn scale(&self) -> f64 {
        self.camera.scale()
    }

    fn px(&self, metres: f64) -> f32 {
        (metres / self.scale()) as f32
    }

    fn visit(&self, id: LocationId, scene: &mut Scene) {
        let u = self.universe;
        let l = u.get(id);
        let Some(world) = anchor(u, id) else {
            return;
        };
        let proj = self.camera.project(world);
        let cluster_px = self.px(self.index.cluster_radius(id));
        let reach = self.camera.viewport.expand(cluster_px + 40.0);
        if !reach.contains(proj.pos) {
            return;
        }
        let expanded = matches!(l.kind, LocationKind::System | LocationKind::Star)
            || u.is_within(self.focus, id)
            || cluster_px > EXPAND_PX;

        if l.kind != LocationKind::System && self.filters.shows(l) {
            let style = kind_style(l.kind);
            let real = match l.kind {
                LocationKind::Star | LocationKind::Planet | LocationKind::Moon => {
                    l.radius_m.map_or(0.0, |r| self.px(r))
                }
                _ => 0.0,
            };
            let collapsed = if expanded {
                0
            } else {
                u.descendants(id)
                    .skip(1)
                    .filter(|d| self.filters.shows(d) && d.position.vec().is_some())
                    .count()
            };
            scene.items.push(SceneItem {
                id,
                pos: proj.pos,
                radius: real.max(style.min_px),
                depth: proj.depth,
                collapsed,
            });
            if self.filters.orbits
                && let Some(center) = orbit_center(u, l)
            {
                let r = (world.x - center.x).hypot(world.y - center.y);
                scene.orbits.extend(self.circle(center, r));
            }
            self.drop_line(l, world, proj.pos, scene);
        }

        if expanded {
            for &c in &l.children {
                self.visit(c, scene);
            }
        }
    }

    /// In the tilted view, a line from objects in open space (and moons) down to their
    /// orbital plane, so heights read correctly.
    fn drop_line(&self, l: &Location, world: Vec3, at: Pos2, scene: &mut Scene) {
        if self.camera.mode != ViewMode::Tilted {
            return;
        }
        let u = self.universe;
        let plane_z = match u.ancestors(l.id).find(|a| a.kind.is_body()) {
            None => 0.0,
            Some(_) if l.kind == LocationKind::Moon => orbit_center(u, l).map_or(0.0, |c| c.z),
            Some(_) => return,
        };
        if self.px((world.z - plane_z).abs()) < 3.0 {
            return;
        }
        let foot = self
            .camera
            .project(Vec3::new(world.x, world.y, plane_z))
            .pos;
        scene.drop_lines.push([at, foot]);
    }

    /// A circle in the XY plane at `center.z`, or `None` when it would be invisible.
    fn circle(&self, center: Vec3, radius: f64) -> Option<Vec<Pos2>> {
        let r_px = self.px(radius);
        if !(3.0..=2e5).contains(&r_px) {
            return None;
        }
        let vp: Rect = self.camera.viewport;
        let half_diag = vp.size().length() / 2.0;
        let c = self.camera.project(center).pos;
        let d = c.distance(vp.center());
        // Entirely outside, or the viewport is entirely inside the circle (top-down).
        if d - r_px > half_diag || (self.camera.mode == ViewMode::TopDown && r_px - d > half_diag) {
            return None;
        }
        // Enough segments to keep the chord error under half a pixel.
        let n = ((std::f32::consts::PI * r_px.sqrt()).ceil() as usize).clamp(48, 2048);
        let points = (0..=n)
            .map(|i| {
                let t = std::f64::consts::TAU * i as f64 / n as f64;
                let p = center + Vec3::new(radius * t.cos(), radius * t.sin(), 0.0);
                self.camera.project(p).pos
            })
            .collect();
        Some(points)
    }

    /// Polar grid on the focus' orbital plane (tilted view only).
    fn grid(&self, scene: &mut Scene) {
        let center = anchor(self.universe, self.focus).unwrap_or(Vec3::ZERO);
        let step = nice_length(self.scale() * 120.0);
        for i in 1..=8 {
            scene.grid.extend(self.circle(center, step * f64::from(i)));
        }
        let outer = step * 8.0;
        for i in 0..12 {
            let t = std::f64::consts::TAU * f64::from(i) / 12.0;
            let tip = center + Vec3::new(outer * t.cos(), outer * t.sin(), 0.0);
            scene.grid.push(vec![
                self.camera.project(center).pos,
                self.camera.project(tip).pos,
            ]);
        }
    }

    fn jumps(&self, scene: &mut Scene) {
        let u = self.universe;
        for link in u.jump_links() {
            for (at, to) in [(link.a, link.b), (link.b, link.a)] {
                if u.get(at).system != self.system {
                    continue;
                }
                let Some(world) = anchor(u, at) else {
                    continue;
                };
                let pos = self.camera.project(world).pos;
                if self.camera.viewport.contains(pos) {
                    scene.jumps.push(JumpMarker { pos, at, to });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use eframe::egui::vec2;
    use sc_data::{LoadOptions, ScunpackedSource, UniverseSource};

    use super::*;

    fn universe() -> Universe {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/sc-data/tests/fixtures/scunpacked");
        ScunpackedSource::from_dir(dir)
            .with_options(LoadOptions::default())
            .load()
            .unwrap_or_else(|e| panic!("fixture load failed: {e}"))
            .universe
    }

    fn id(u: &Universe, name: &str) -> LocationId {
        match u.find_exact(name).as_slice() {
            [m] => m.id,
            other => panic!("expected one {name:?}, found {}", other.len()),
        }
    }

    fn names(u: &Universe, scene: &Scene) -> Vec<String> {
        scene
            .items
            .iter()
            .map(|it| u.get(it.id).name.clone())
            .collect()
    }

    /// Scene with the camera framed on `focus` the way the map does it.
    fn scene_at(u: &Universe, focus: LocationId, filters: &Filters, mode: ViewMode) -> Scene {
        let index = MapIndex::new(u);
        let mut camera = Camera::default();
        camera.mode = mode;
        camera.viewport = Rect::from_min_size(Pos2::ZERO, vec2(1200.0, 800.0));
        let center = anchor(u, focus).unwrap_or(Vec3::ZERO);
        camera.frame_now(center, index.cluster_radius(focus));
        SceneBuilder {
            universe: u,
            index: &index,
            camera: &camera,
            filters,
            system: u.get(focus).system,
            focus,
        }
        .build()
    }

    #[test]
    fn system_overview_collapses_planet_surfaces() {
        let u = universe();
        let stanton = u
            .system("Stanton")
            .map(|s| s.id)
            .unwrap_or_else(|| panic!());
        let scene = scene_at(&u, stanton, &Filters::default(), ViewMode::TopDown);
        let shown = names(&u, &scene);
        for expected in ["Stanton", "Hurston", "ArcCorp", "microTech", "Pyro Gateway"] {
            assert!(
                shown.iter().any(|n| n == expected),
                "{expected} in {shown:?}"
            );
        }
        assert!(!shown.iter().any(|n| n == "Lorville"), "{shown:?}");
        let hurston = scene
            .item(id(&u, "Hurston"))
            .unwrap_or_else(|| panic!("no Hurston"));
        assert!(hurston.collapsed > 0);
        assert!(!scene.orbits.is_empty());
        assert!(!scene.jumps.is_empty(), "the Pyro jump point is on screen");
    }

    #[test]
    fn focusing_a_planet_shows_its_moons_and_cities() {
        let u = universe();
        let hurston = id(&u, "Hurston");
        for mode in [ViewMode::TopDown, ViewMode::Tilted] {
            let scene = scene_at(&u, hurston, &Filters::default(), mode);
            let shown = names(&u, &scene);
            for expected in ["Hurston", "Lorville", "Arial", "Everus Harbor"] {
                assert!(
                    shown.iter().any(|n| n == expected),
                    "{expected} in {shown:?}"
                );
            }
            assert_eq!(scene.item(hurston).map(|it| it.collapsed), Some(0));
        }
    }

    #[test]
    fn filters_remove_items() {
        let u = universe();
        let hurston = id(&u, "Hurston");
        let filters = Filters {
            hidden_kinds: HashSet::from([LocationKind::City, LocationKind::Moon]),
            show_hidden: false,
            ..Filters::default()
        };
        let scene = scene_at(&u, hurston, &filters, ViewMode::TopDown);
        for it in &scene.items {
            let l = u.get(it.id);
            assert!(!matches!(l.kind, LocationKind::City | LocationKind::Moon));
            assert!(!l.hidden, "{}", l.name);
        }
    }

    #[test]
    fn only_positioned_locations_are_drawn() {
        let u = universe();
        let pyro = u.system("Pyro").map(|s| s.id).unwrap_or_else(|| panic!());
        let scene = scene_at(&u, pyro, &Filters::default(), ViewMode::TopDown);
        assert!(!scene.items.is_empty());
        for it in &scene.items {
            assert!(u.get(it.id).position.vec().is_some());
        }
    }

    #[test]
    fn picks_the_closest_marker() {
        let u = universe();
        let hurston = id(&u, "Hurston");
        let scene = scene_at(&u, hurston, &Filters::default(), ViewMode::TopDown);
        let lorville = scene
            .item(id(&u, "Lorville"))
            .unwrap_or_else(|| panic!("no Lorville"));
        assert_eq!(scene.pick(lorville.pos).map(|it| it.id), Some(lorville.id));
    }

    #[test]
    fn cluster_radius_covers_children() {
        let u = universe();
        let index = MapIndex::new(&u);
        let hurston = id(&u, "Hurston");
        let arial = id(&u, "Arial");
        let d = anchor(&u, hurston)
            .zip(anchor(&u, arial))
            .map(|(a, b)| a.distance_to(b))
            .unwrap_or_else(|| panic!("no positions"));
        assert!(index.cluster_radius(hurston) >= d);
    }
}
