use crate::{GameUuid, LocationId, LocationKind, Vec3};

/// Where a location is, in its system's frame (metres).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Position {
    /// Coordinates come straight from the data source.
    Exact(Vec3),
    /// The source had no coordinates; this is the position of the closest ancestor
    /// (`from`) that had some. Good enough for inter-body distances.
    Inherited {
        from: LocationId,
        pos: Vec3,
    },
    Unknown,
}

impl Position {
    pub fn vec(self) -> Option<Vec3> {
        match self {
            Self::Exact(v) | Self::Inherited { pos: v, .. } => Some(v),
            Self::Unknown => None,
        }
    }

    pub fn is_exact(self) -> bool {
        matches!(self, Self::Exact(_))
    }
}

/// A service available at a location (`Commodity Trading - Freight Elevator`, `Refinery`...).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Amenity {
    /// Internal name from the game files, stable across languages.
    pub key: String,
    /// Player-facing label.
    pub display: String,
}

/// One node of the universe tree.
#[derive(Debug, Clone)]
pub struct Location {
    pub id: LocationId,
    /// Game object id. `None` for synthetic nodes (systems).
    pub uuid: Option<GameUuid>,
    pub name: String,
    pub kind: LocationKind,
    /// Type as named by the source (e.g. `Manmade_VisibleOnInteraction`).
    pub raw_type: String,
    /// `None` only for [`LocationKind::System`] roots.
    pub parent: Option<LocationId>,
    pub children: Vec<LocationId>,
    /// The [`LocationKind::System`] root this location belongs to.
    pub system: LocationId,
    pub position: Position,
    /// Body radius in metres for planets/moons/stars, object size otherwise.
    pub radius_m: Option<f64>,
    /// Can be targeted by quantum travel.
    pub qt_valid: bool,
    /// Hidden on the in-game starmap until discovered / interacted with.
    pub hidden: bool,
    /// Game hierarchy tag (`Stanton1`, `Pyro3`, ...).
    pub hierarchy_tag: Option<String>,
    pub jurisdiction: Option<String>,
    pub amenities: Vec<Amenity>,
}

impl Location {
    /// True if the location has a commodity trading terminal (freight elevator or loading dock).
    pub fn has_commodity_trading(&self) -> bool {
        self.amenities
            .iter()
            .any(|a| a.key.starts_with("Commodity Trading"))
    }

    pub fn has_amenity(&self, key: &str) -> bool {
        self.amenities.iter().any(|a| a.key == key)
    }
}
