//! Draws a [`Scene`] with an egui painter.

use eframe::egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Shape, Stroke, pos2, vec2};
use sc_universe::{Distance, LocationId, Position, Universe};

use super::camera::{Camera, nice_length};
use super::scene::{Scene, SceneItem};
use super::style::{self, Marker, kind_style};

/// Most labels laid out per frame; the rest would be hidden by decluttering anyway.
const MAX_LABELS: usize = 400;

pub struct MapPainter<'a> {
    pub painter: &'a Painter,
    pub universe: &'a Universe,
    pub scene: &'a Scene,
    pub camera: &'a Camera,
    pub focus: LocationId,
    pub selected: Option<LocationId>,
    pub hovered: Option<LocationId>,
    pub highlight_trading: bool,
    pub labels: bool,
}

impl MapPainter<'_> {
    /// Paints everything. Returns the clickable jump labels and the system they lead to.
    pub fn paint(&self) -> Vec<(Rect, LocationId)> {
        let p = self.painter;
        for line in &self.scene.grid {
            p.add(Shape::line(line.clone(), Stroke::new(1.0_f32, style::GRID)));
        }
        for orbit in &self.scene.orbits {
            p.add(Shape::line(
                orbit.clone(),
                Stroke::new(1.0_f32, style::ORBIT),
            ));
        }
        for line in &self.scene.drop_lines {
            p.line_segment(*line, Stroke::new(1.0_f32, style::ORBIT));
        }
        for item in &self.scene.items {
            self.item(item);
        }
        let jumps = self.jump_labels();
        if self.labels {
            self.labels();
        }
        self.scale_bar();
        jumps
    }

    fn item(&self, it: &SceneItem) {
        let p = self.painter;
        let l = self.universe.get(it.id);
        let st = kind_style(l.kind);
        let color = if l.hidden {
            st.color.gamma_multiply(0.45)
        } else {
            st.color
        };
        let hollow = matches!(l.position, Position::Inherited { .. });
        let fill = if hollow { Color32::TRANSPARENT } else { color };
        let stroke = Stroke::new(1.5_f32, color);
        let (c, r) = (it.pos, it.radius);
        match st.marker {
            // Bodies shown at their real size are translucent so what's on them stays visible.
            Marker::Disc if r > st.min_px + 0.5 => {
                p.circle(c, r, color.gamma_multiply(0.18), stroke);
            }
            Marker::Disc => {
                p.circle(c, r, fill, stroke);
            }
            Marker::Square => {
                let pts = vec![
                    c + vec2(-r, -r),
                    c + vec2(r, -r),
                    c + vec2(r, r),
                    c + vec2(-r, r),
                ];
                p.add(Shape::convex_polygon(pts, fill, stroke));
            }
            Marker::Diamond => {
                let k = r * 1.3;
                let pts = vec![
                    c + vec2(0.0, -k),
                    c + vec2(k, 0.0),
                    c + vec2(0.0, k),
                    c + vec2(-k, 0.0),
                ];
                p.add(Shape::convex_polygon(pts, fill, stroke));
            }
            Marker::Triangle => {
                let k = r * 1.3;
                let pts = vec![
                    c + vec2(0.0, -k),
                    c + vec2(k * 0.87, k * 0.5),
                    c + vec2(-k * 0.87, k * 0.5),
                ];
                p.add(Shape::convex_polygon(pts, fill, stroke));
            }
            Marker::Ring => {
                p.circle_stroke(c, r, Stroke::new(2.0_f32, color));
                p.circle_filled(c, 1.5, color);
            }
        }
        if self.highlight_trading && l.has_commodity_trading() {
            p.circle_stroke(c, r + 3.0, Stroke::new(1.2_f32, style::TRADING));
        }
        if self.selected == Some(it.id) {
            p.circle_stroke(c, r + 6.0, Stroke::new(2.0_f32, style::SELECTED));
        } else if self.hovered == Some(it.id) {
            p.circle_stroke(
                c,
                r + 5.0,
                Stroke::new(1.0_f32, style::SELECTED.gamma_multiply(0.6)),
            );
        }
    }

    fn label_priority(&self, it: &SceneItem) -> u32 {
        let l = self.universe.get(it.id);
        if self.selected == Some(it.id) {
            1000
        } else if self.hovered == Some(it.id) {
            999
        } else if it.id == self.focus {
            998
        } else {
            let near_focus = l.parent == Some(self.focus);
            u32::from(kind_style(l.kind).label_priority) * 10
                + u32::from(near_focus) * 5
                + u32::from(!l.hidden) * 2
                + u32::from(it.collapsed > 0)
        }
    }

    /// Labels by priority, skipping any that would overlap one already placed.
    fn labels(&self) {
        let p = self.painter;
        let mut order: Vec<&SceneItem> = self.scene.items.iter().collect();
        order.sort_by_key(|it| std::cmp::Reverse(self.label_priority(it)));
        let mut placed: Vec<Rect> = Vec::new();
        for it in order.into_iter().take(MAX_LABELS) {
            let l = self.universe.get(it.id);
            let mut text = l.name.clone();
            if it.collapsed > 0 {
                text.push_str(&format!("  +{}", it.collapsed));
            }
            let important = self.selected == Some(it.id) || self.hovered == Some(it.id);
            let color = if important {
                Color32::WHITE
            } else if l.hidden {
                Color32::from_gray(130)
            } else {
                Color32::from_gray(200)
            };
            let galley = p.layout_no_wrap(text, FontId::proportional(13.0), color);
            let min = it.pos + vec2(it.radius + 4.0, -galley.size().y / 2.0);
            let rect = Rect::from_min_size(min, galley.size()).expand(1.0);
            if !self.camera.viewport.intersects(rect)
                || (!important && placed.iter().any(|r| r.intersects(rect)))
            {
                continue;
            }
            placed.push(rect);
            p.galley(min, galley, color);
        }
    }

    fn jump_labels(&self) -> Vec<(Rect, LocationId)> {
        let u = self.universe;
        self.scene
            .jumps
            .iter()
            .map(|j| {
                let to_system = u.get(u.get(j.to).system);
                let rect = self.painter.text(
                    j.pos + vec2(8.0, 10.0),
                    Align2::LEFT_TOP,
                    format!("» {}", to_system.name),
                    FontId::proportional(13.0),
                    style::JUMP,
                );
                (rect.expand(3.0), j.to)
            })
            .collect()
    }

    fn scale_bar(&self) {
        let p = self.painter;
        let vp = self.camera.viewport;
        let len = nice_length(self.camera.scale() * 120.0);
        let px = (len / self.camera.scale()) as f32;
        let y = vp.bottom() - 18.0;
        let (a, b) = (pos2(vp.left() + 16.0, y), pos2(vp.left() + 16.0 + px, y));
        let stroke = Stroke::new(1.5_f32, Color32::from_gray(180));
        p.line_segment([a, b], stroke);
        for x in [a, b] {
            p.line_segment([x - vec2(0.0, 4.0), x + vec2(0.0, 4.0)], stroke);
        }
        p.text(
            pos2((a.x + b.x) / 2.0, y - 6.0),
            Align2::CENTER_BOTTOM,
            Distance(len).to_string(),
            FontId::proportional(12.0),
            Color32::from_gray(180),
        );
    }
}

/// A small coloured dot for legends.
pub fn swatch(painter: &Painter, center: Pos2, kind: sc_universe::LocationKind) {
    painter.circle_filled(center, 4.0, kind_style(kind).color);
}
