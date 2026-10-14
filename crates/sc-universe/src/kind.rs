use std::fmt;

use serde::{Deserialize, Serialize};

/// What a location is, normalised from the game's starmap object type.
///
/// The raw game type is always kept in [`crate::Location::raw_type`]; this enum is the
/// coarse classification the rest of the app reasons about.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum LocationKind {
    /// Synthetic root for a star system (not a game object).
    System,
    Star,
    Planet,
    Moon,
    /// Space station, rest stop, L-point station, orbital platform.
    Station,
    /// Major landing zone (Lorville, Area18, ...). Game type `LandingZone`.
    City,
    Outpost,
    PointOfInterest,
    /// Quantum-reachable asteroid (bases, clusters). Game type `Asteroid_ValidQT`.
    AsteroidBase,
    /// Asteroid scatter, usually hidden and not quantum-reachable.
    Asteroid,
    /// Station that hosts a jump point (`Stanton Gateway`, ...).
    Gateway,
    /// The jump point itself.
    JumpPoint,
    CommArray,
    NavPoint,
    Anomaly,
    Other,
}

impl LocationKind {
    /// Maps a raw starmap type (`Type.Name`) and display name to a kind.
    ///
    /// Returns `None` for starmap UI helpers that are not locations
    /// (`SolarSystem`, `YouAreHere`, `CardinalPoint`, `QuantumTracePoint`).
    pub fn from_raw(raw_type: &str, name: &str) -> Option<Self> {
        let kind = match raw_type {
            "Star" => Self::Star,
            "Planet" => Self::Planet,
            "Moon" => Self::Moon,
            "LandingZone" => Self::City,
            "Manmade" | "Manmade_VisibleOnInteraction" => {
                if name.ends_with("Gateway") {
                    Self::Gateway
                } else if name.starts_with("Comm Array") {
                    Self::CommArray
                } else {
                    Self::Station
                }
            }
            "Outpost" | "Outpost_InvalidQT" => Self::Outpost,
            "PointOfInterest" => Self::PointOfInterest,
            "Asteroid_ValidQT" => Self::AsteroidBase,
            "Asteroid" => Self::Asteroid,
            "JumpPoint" => Self::JumpPoint,
            "Anomaly" if name.contains("Jump Point") => Self::JumpPoint,
            "Anomaly" => Self::Anomaly,
            "NavPoint" => Self::NavPoint,
            "SolarSystem" | "YouAreHere" | "CardinalPoint" | "QuantumTracePoint" => return None,
            _ => Self::Other,
        };
        Some(kind)
    }

    /// Celestial bodies a location can sit on or orbit.
    pub fn is_body(self) -> bool {
        matches!(self, Self::Planet | Self::Moon)
    }

    /// Sort rank used to order children in a stable, readable way.
    pub(crate) fn rank(self) -> u8 {
        self as u8
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::System => "System",
            Self::Star => "Star",
            Self::Planet => "Planet",
            Self::Moon => "Moon",
            Self::Station => "Station",
            Self::City => "City",
            Self::Outpost => "Outpost",
            Self::PointOfInterest => "POI",
            Self::AsteroidBase => "Asteroid base",
            Self::Asteroid => "Asteroid",
            Self::Gateway => "Gateway",
            Self::JumpPoint => "Jump point",
            Self::CommArray => "Comm array",
            Self::NavPoint => "Nav point",
            Self::Anomaly => "Anomaly",
            Self::Other => "Other",
        }
    }
}

impl fmt::Display for LocationKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.label())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn maps_manmade_by_name() {
        assert_eq!(
            LocationKind::from_raw("Manmade", "Pyro Gateway"),
            Some(LocationKind::Gateway)
        );
        assert_eq!(
            LocationKind::from_raw("Manmade", "Comm Array ST1-61"),
            Some(LocationKind::CommArray)
        );
        assert_eq!(
            LocationKind::from_raw("Manmade_VisibleOnInteraction", "HUR-L1 Green Glade Station"),
            Some(LocationKind::Station)
        );
    }

    #[test]
    fn maps_anomaly_jump_points() {
        assert_eq!(
            LocationKind::from_raw("Anomaly", "Pyro-Stanton Jump Point"),
            Some(LocationKind::JumpPoint)
        );
        assert_eq!(
            LocationKind::from_raw("Anomaly", "Weird"),
            Some(LocationKind::Anomaly)
        );
    }

    #[test]
    fn drops_ui_types() {
        assert_eq!(
            LocationKind::from_raw("SolarSystem", "Stanton System"),
            None
        );
        assert_eq!(LocationKind::from_raw("YouAreHere", "x"), None);
    }

    #[test]
    fn unknown_types_are_other() {
        assert_eq!(
            LocationKind::from_raw("unknown", "x"),
            Some(LocationKind::Other)
        );
    }
}
