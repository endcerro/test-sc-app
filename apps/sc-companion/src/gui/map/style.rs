//! How each [`LocationKind`] looks on the map.

use eframe::egui::Color32;
use sc_universe::LocationKind;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Marker {
    Disc,
    Square,
    Diamond,
    Triangle,
    Ring,
}

#[derive(Debug, Clone, Copy)]
pub struct KindStyle {
    pub color: Color32,
    pub marker: Marker,
    /// Marker radius when the object's real size is smaller on screen.
    pub min_px: f32,
    /// Higher wins when labels collide.
    pub label_priority: u8,
}

/// Kinds that can be toggled on the map, in legend order.
pub const LAYERS: [LocationKind; 15] = [
    LocationKind::Star,
    LocationKind::Planet,
    LocationKind::Moon,
    LocationKind::City,
    LocationKind::Station,
    LocationKind::Gateway,
    LocationKind::JumpPoint,
    LocationKind::Outpost,
    LocationKind::PointOfInterest,
    LocationKind::AsteroidBase,
    LocationKind::CommArray,
    LocationKind::NavPoint,
    LocationKind::Anomaly,
    LocationKind::Asteroid,
    LocationKind::Other,
];

pub const TRADING: Color32 = Color32::from_rgb(110, 220, 120);
pub const SELECTED: Color32 = Color32::WHITE;
pub const ORBIT: Color32 = Color32::from_rgba_premultiplied(70, 80, 100, 110);
pub const GRID: Color32 = Color32::from_rgba_premultiplied(40, 50, 64, 90);
pub const JUMP: Color32 = Color32::from_rgb(255, 110, 210);

pub fn kind_style(kind: LocationKind) -> KindStyle {
    let (rgb, marker, min_px, label_priority) = match kind {
        LocationKind::System | LocationKind::Star => ((255, 210, 90), Marker::Disc, 7.0, 10),
        LocationKind::Planet => ((90, 150, 235), Marker::Disc, 5.5, 9),
        LocationKind::Moon => ((160, 168, 182), Marker::Disc, 4.0, 8),
        LocationKind::City => ((245, 145, 60), Marker::Square, 4.5, 7),
        LocationKind::Gateway => ((230, 90, 220), Marker::Ring, 5.0, 7),
        LocationKind::JumpPoint => ((255, 110, 210), Marker::Ring, 5.0, 6),
        LocationKind::Station => ((80, 205, 205), Marker::Square, 3.5, 6),
        LocationKind::AsteroidBase => ((190, 155, 110), Marker::Diamond, 3.5, 5),
        LocationKind::Outpost => ((175, 205, 95), Marker::Triangle, 3.5, 4),
        LocationKind::PointOfInterest => ((205, 125, 205), Marker::Diamond, 3.5, 3),
        LocationKind::CommArray => ((135, 135, 225), Marker::Triangle, 3.0, 2),
        LocationKind::Anomaly => ((230, 90, 90), Marker::Diamond, 3.0, 2),
        LocationKind::NavPoint => ((150, 150, 150), Marker::Disc, 2.5, 1),
        LocationKind::Asteroid => ((125, 115, 105), Marker::Disc, 2.0, 0),
        LocationKind::Other => ((140, 140, 140), Marker::Disc, 2.5, 0),
    };
    KindStyle {
        color: Color32::from_rgb(rgb.0, rgb.1, rgb.2),
        marker,
        min_px,
        label_priority,
    }
}
