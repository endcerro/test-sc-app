//! Serde mirrors of the scunpacked-data files.
//!
//! Only the fields we use are declared; unknown fields are ignored and most fields default,
//! so upstream additions or omissions don't break loading.

use std::collections::BTreeMap;
use std::fmt;

use serde::de::{IgnoredAny, MapAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize};

/// One entry of `starmap.json`.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StarmapObject {
    #[serde(rename = "UUID")]
    pub uuid: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "ParentUUID", default)]
    pub parent_uuid: Option<String>,
    #[serde(rename = "Type", default)]
    pub kind: Option<StarmapType>,
    /// Radius in metres for bodies; object size otherwise.
    #[serde(default)]
    pub size: Option<f64>,
    #[serde(default)]
    pub location_hierarchy_tag: Option<NamedRef>,
    #[serde(default)]
    pub jurisdiction: Option<NamedRef>,
    #[serde(default)]
    pub amenities: Vec<AmenityDto>,
    #[serde(default)]
    pub hide_in_starmap: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct NamedRef {
    #[serde(default)]
    pub name: Option<String>,
}

/// `Type` of a starmap object.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct StarmapType {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub valid_quantum_travel_destination: bool,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "PascalCase")]
pub struct AmenityDto {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub display_name: Option<String>,
}

/// `starmap_positions.json`.
#[derive(Debug, Clone, Default, Deserialize)]
pub struct PositionsFile {
    #[serde(default)]
    pub entities: Vec<PositionEntity>,
    #[serde(default)]
    pub connections: Vec<Connection>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct PositionEntity {
    pub uuid: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(rename = "type", default)]
    pub kind: Option<String>,
    /// Lowercase system code: `stanton`, `pyro`, `nyx`.
    #[serde(default)]
    pub system: Option<String>,
    #[serde(default)]
    pub parent_uuid: Option<String>,
    #[serde(default)]
    pub hidden: bool,
    #[serde(default)]
    pub qt_valid: bool,
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

/// A jump tunnel between two jump point objects.
#[derive(Debug, Clone, Deserialize)]
pub struct Connection {
    pub entry_uuid: String,
    pub exit_uuid: String,
    #[serde(default)]
    pub fuel_cost: Option<f64>,
}

/// The `mission_location_*` entries of `labels.json` (the game's English localization),
/// e.g. `"Riker Memorial Spaceport in Area18"`.
///
/// Deserializing skips every other key, so the 11 MB `labels.json` and the small
/// `mission_locations.json` written to the cache parse into the same type.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct MissionLocations(pub BTreeMap<String, String>);

impl MissionLocations {
    pub const KEY_PREFIX: &'static str = "mission_location_";
}

impl<'de> Deserialize<'de> for MissionLocations {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct Visit;
        impl<'de> Visitor<'de> for Visit {
            type Value = MissionLocations;

            fn expecting(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str("a map of localization keys to strings")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let mut out = BTreeMap::new();
                while let Some(key) = map.next_key::<String>()? {
                    if key.starts_with(MissionLocations::KEY_PREFIX) {
                        if let serde_json::Value::String(v) = map.next_value()? {
                            out.insert(key, v);
                        }
                    } else {
                        map.next_value::<IgnoredAny>()?;
                    }
                }
                Ok(MissionLocations(out))
            }
        }
        deserializer.deserialize_map(Visit)
    }
}
