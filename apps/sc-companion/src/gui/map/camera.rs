//! Map camera: orthographic projection from a system frame (metres, f64) to screen pixels.
//!
//! All world maths is done in f64 relative to [`View::center`] and only converted to f32
//! once scaled to pixels: systems span ~10¹¹ m, where f32 would lose kilometres.

use eframe::egui::{Pos2, Rect, Vec2, vec2};
use sc_universe::Vec3;

/// Closest and farthest zoom, in metres per pixel.
pub const MIN_SCALE: f64 = 5.0;
pub const MAX_SCALE: f64 = 1e9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    /// Looking down the system's Z axis.
    TopDown,
    /// Orbit camera: yaw around Z, then pitch towards the orbital plane.
    Tilted,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct View {
    /// World point at the middle of the viewport.
    pub center: Vec3,
    /// Metres per pixel.
    pub scale: f64,
    /// Rotation around the system's Z axis, radians (tilted mode only).
    pub yaw: f64,
    /// 0 = top-down, π/2 = edge-on (tilted mode only).
    pub pitch: f64,
}

/// A projected point: screen position plus how close it is to the viewer.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Projected {
    pub pos: Pos2,
    /// Larger is nearer; draw in increasing order.
    pub depth: f64,
}

#[derive(Debug, Clone)]
pub struct Camera {
    pub mode: ViewMode,
    pub viewport: Rect,
    view: View,
    target: Option<View>,
}

impl Default for Camera {
    fn default() -> Self {
        Self {
            mode: ViewMode::TopDown,
            viewport: Rect::from_min_size(Pos2::ZERO, vec2(800.0, 600.0)),
            view: View {
                center: Vec3::ZERO,
                scale: 1e8,
                yaw: 0.0,
                pitch: 55f64.to_radians(),
            },
            target: None,
        }
    }
}

impl Camera {
    pub fn scale(&self) -> f64 {
        self.view.scale
    }

    #[cfg(test)]
    pub fn center(&self) -> Vec3 {
        self.view.center
    }

    fn angles(&self) -> (f64, f64) {
        match self.mode {
            ViewMode::TopDown => (0.0, 0.0),
            ViewMode::Tilted => (self.view.yaw, self.view.pitch),
        }
    }

    /// World directions of the screen's right and up axes.
    fn axes(&self) -> (Vec3, Vec3) {
        let (yaw, pitch) = self.angles();
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        (Vec3::new(cy, -sy, 0.0), Vec3::new(sy * cp, cy * cp, sp))
    }

    pub fn project(&self, world: Vec3) -> Projected {
        let d = world - self.view.center;
        let (yaw, pitch) = self.angles();
        let (sy, cy) = yaw.sin_cos();
        let (sp, cp) = pitch.sin_cos();
        let x1 = d.x * cy - d.y * sy;
        let y1 = d.x * sy + d.y * cy;
        let up = y1 * cp + d.z * sp;
        let depth = d.z * cp - y1 * sp;
        let s = self.view.scale;
        Projected {
            pos: self.viewport.center() + vec2((x1 / s) as f32, (-up / s) as f32),
            depth,
        }
    }

    /// The world point under a screen position, on the screen-parallel plane through the
    /// view center.
    pub fn unproject(&self, screen: Pos2) -> Vec3 {
        self.view.center + self.screen_offset_to_world(screen - self.viewport.center())
    }

    fn screen_offset_to_world(&self, offset: Vec2) -> Vec3 {
        let (right, up) = self.axes();
        let s = self.view.scale;
        let (dx, dy) = (f64::from(offset.x) * s, -f64::from(offset.y) * s);
        Vec3::new(
            right.x * dx + up.x * dy,
            right.y * dx + up.y * dy,
            right.z * dx + up.z * dy,
        )
    }

    /// Moves the view so the content follows a drag of `delta` pixels.
    pub fn pan(&mut self, delta: Vec2) {
        self.target = None;
        self.view.center = self.view.center - self.screen_offset_to_world(delta);
    }

    /// Zooms by `factor` (> 1 zooms in) keeping the world point under `cursor` fixed.
    pub fn zoom_at(&mut self, cursor: Pos2, factor: f64) {
        self.target = None;
        let anchor = self.unproject(cursor);
        self.view.scale = (self.view.scale / factor).clamp(MIN_SCALE, MAX_SCALE);
        self.view.center = anchor - self.screen_offset_to_world(cursor - self.viewport.center());
    }

    /// Rotates the tilted view by a drag of `delta` pixels.
    pub fn orbit(&mut self, delta: Vec2) {
        if self.mode != ViewMode::Tilted {
            return;
        }
        let v = &mut self.view;
        v.yaw = (v.yaw + f64::from(delta.x) * 0.008).rem_euclid(std::f64::consts::TAU);
        v.pitch = (v.pitch - f64::from(delta.y) * 0.008).clamp(0.0, 89f64.to_radians());
        if let Some(t) = &mut self.target {
            t.yaw = v.yaw;
            t.pitch = v.pitch;
        }
    }

    /// The view that shows a sphere of `extent` metres around `center` in ~80 % of the
    /// viewport.
    fn framing(&self, center: Vec3, extent: f64) -> View {
        let px = f64::from(self.viewport.width().min(self.viewport.height())).max(100.0);
        View {
            center,
            scale: (2.0 * extent / (0.8 * px)).clamp(MIN_SCALE, MAX_SCALE),
            ..self.view
        }
    }

    /// Animates towards a view of `extent` metres around `center`.
    pub fn frame(&mut self, center: Vec3, extent: f64) {
        self.target = Some(self.framing(center, extent));
    }

    /// Jumps to a view of `extent` metres around `center`.
    pub fn frame_now(&mut self, center: Vec3, extent: f64) {
        self.view = self.framing(center, extent);
        self.target = None;
    }

    /// Advances the framing animation by `dt` seconds. Returns true while moving.
    pub fn animate(&mut self, dt: f64) -> bool {
        let Some(target) = self.target else {
            return false;
        };
        // Exponential ease; ~95 % of the way after 0.35 s.
        let t = 1.0 - (-dt / 0.12).exp();
        let v = &mut self.view;
        let ln_scale = v.scale.ln() + (target.scale.ln() - v.scale.ln()) * t;
        v.scale = ln_scale.exp();
        v.center = v.center + scaled(target.center - v.center, t);
        let close = (target.scale / v.scale).ln().abs() < 1e-3
            && target.center.distance_to(v.center) < v.scale * 0.25;
        if close {
            *v = target;
            self.target = None;
        }
        true
    }
}

fn scaled(v: Vec3, k: f64) -> Vec3 {
    Vec3::new(v.x * k, v.y * k, v.z * k)
}

/// A "round" length (1, 2 or 5 × 10ⁿ metres) close to `target` metres.
pub fn nice_length(target: f64) -> f64 {
    let exp = target.log10().floor();
    let base = 10f64.powf(exp);
    let m = target / base;
    let nice = if m < 1.5 {
        1.0
    } else if m < 3.5 {
        2.0
    } else if m < 7.5 {
        5.0
    } else {
        10.0
    };
    nice * base
}

#[cfg(test)]
mod tests {
    use eframe::egui::pos2 as screen;

    use super::*;

    fn camera(mode: ViewMode) -> Camera {
        let mut c = Camera {
            mode,
            viewport: Rect::from_min_size(screen(0.0, 0.0), vec2(1000.0, 800.0)),
            ..Camera::default()
        };
        c.view.yaw = 0.7;
        c.view.pitch = 0.4;
        c.frame_now(Vec3::new(1e10, -3e9, 0.0), 1e9);
        c
    }

    fn close(a: Pos2, b: Pos2) -> bool {
        (a - b).length() < 1e-3
    }

    #[test]
    fn unproject_round_trips() {
        for mode in [ViewMode::TopDown, ViewMode::Tilted] {
            let c = camera(mode);
            for p in [screen(10.0, 20.0), screen(500.0, 400.0), screen(990.0, 5.0)] {
                assert!(close(c.project(c.unproject(p)).pos, p), "{mode:?} {p:?}");
            }
        }
    }

    #[test]
    fn top_down_maps_y_up() {
        let c = camera(ViewMode::TopDown);
        let center = c.project(c.center()).pos;
        assert!(close(center, c.viewport.center()));
        let north = c
            .project(c.center() + Vec3::new(0.0, c.scale() * 10.0, 0.0))
            .pos;
        assert!(close(north, center - vec2(0.0, 10.0)));
    }

    #[test]
    fn zoom_keeps_cursor_point_fixed() {
        for mode in [ViewMode::TopDown, ViewMode::Tilted] {
            let mut c = camera(mode);
            let cursor = screen(700.0, 150.0);
            let world = c.unproject(cursor);
            c.zoom_at(cursor, 3.0);
            assert!(close(c.project(world).pos, cursor), "{mode:?}");
        }
    }

    #[test]
    fn pan_follows_drag() {
        let mut c = camera(ViewMode::Tilted);
        let world = c.unproject(screen(300.0, 300.0));
        c.pan(vec2(25.0, -40.0));
        assert!(close(c.project(world).pos, screen(325.0, 260.0)));
    }

    #[test]
    fn keeps_metre_precision_far_from_origin() {
        let mut c = camera(ViewMode::TopDown);
        c.frame_now(Vec3::new(4e10, 2e10, 0.0), 100.0);
        assert!(c.scale() < 1.0 || c.scale() == MIN_SCALE);
        let a = c.project(Vec3::new(4e10, 2e10, 0.0)).pos;
        let b = c.project(Vec3::new(4e10 + 50.0, 2e10, 0.0)).pos;
        assert!((b.x - a.x - (50.0 / c.scale()) as f32).abs() < 1e-3);
    }

    #[test]
    fn animation_reaches_target() {
        let mut c = camera(ViewMode::TopDown);
        let goal = Vec3::new(-2e10, 5e9, 1e6);
        c.frame(goal, 2e6);
        let mut steps = 0;
        while c.animate(1.0 / 60.0) {
            steps += 1;
            assert!(steps < 600, "animation never settles");
        }
        assert_eq!(c.center(), goal);
        assert!((c.scale() - 2.0 * 2e6 / (0.8 * 800.0)).abs() < 1e-6);
    }

    #[test]
    fn nice_lengths() {
        assert_eq!(nice_length(1.2e9), 1e9);
        assert_eq!(nice_length(2.9e3), 2e3);
        assert_eq!(nice_length(6.0), 5.0);
        assert_eq!(nice_length(8.0e5), 1e6);
    }
}
