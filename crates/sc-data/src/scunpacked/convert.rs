//! Merge of `starmap.json` (tree + metadata), `starmap_positions.json` (coordinates,
//! system codes, jump connections) and aliases (curated + `labels.json`) into a
//! [`UniverseBuilder`].

use std::collections::{HashMap, HashSet};

use sc_universe::{Amenity, GameUuid, JumpSpec, LocationKind, NodeSpec, UniverseBuilder, Vec3};

use super::aliases::{self, Targets};
use super::dto::{MissionLocations, PositionEntity, PositionsFile, StarmapObject};
use crate::LoadOptions;

/// True for names the game files leave unset (`<= PLACEHOLDER =>`, `@loc_key`, empty).
pub(crate) fn is_placeholder(name: &str) -> bool {
    let n = name.trim();
    n.is_empty()
        || n.starts_with("<=")
        || n.starts_with('@')
        || n.contains("PLACEHOLDER")
        || n.contains("UNINITIALIZED")
}

fn parse_uuid(s: &str) -> Option<GameUuid> {
    match GameUuid::parse(s) {
        Ok(u) => Some(u),
        Err(e) => {
            tracing::debug!("skipping invalid uuid {s:?}: {e}");
            None
        }
    }
}

/// Returns the builder plus notes worth surfacing (curated alias drift).
pub(crate) fn build(
    starmap: &[StarmapObject],
    positions: &PositionsFile,
    labels: Option<&MissionLocations>,
    opts: LoadOptions,
) -> (UniverseBuilder, Vec<String>) {
    // First row wins: duplicates are asteroid scatter sharing one template uuid.
    let mut pos: HashMap<&str, &PositionEntity> = HashMap::with_capacity(positions.entities.len());
    for e in &positions.entities {
        pos.entry(e.uuid.as_str()).or_insert(e);
    }

    // Decide which starmap objects become nodes.
    let mut kept: HashSet<&str> = HashSet::new();
    let mut specs: Vec<(NodeSpec, Option<&str>)> = Vec::new();
    for obj in starmap {
        let Some(name) = obj.name.as_deref().filter(|n| !is_placeholder(n)) else {
            continue;
        };
        let raw_type = obj
            .kind
            .as_ref()
            .and_then(|t| t.name.as_deref())
            .unwrap_or("unknown");
        let Some(kind) = LocationKind::from_raw(raw_type, name) else {
            continue;
        };
        if kind == LocationKind::Asteroid && !opts.include_asteroid_scatter {
            continue;
        }
        let Some(uuid) = parse_uuid(&obj.uuid) else {
            continue;
        };
        let p = pos.get(obj.uuid.as_str()).copied();
        if obj.hide_in_starmap
            && p.is_none()
            && obj.amenities.is_empty()
            && !opts.include_hidden_unpositioned
        {
            continue;
        }
        let mut spec = NodeSpec::new(uuid, name, kind);
        spec.raw_type = raw_type.to_owned();
        spec.system = p.and_then(|p| p.system.clone());
        spec.position = p.map(|p| Vec3::new(p.x, p.y, p.z));
        spec.radius_m = obj.size.filter(|s| *s > 0.0);
        spec.qt_valid = obj
            .kind
            .as_ref()
            .map(|t| t.valid_quantum_travel_destination)
            .unwrap_or_default()
            || p.is_some_and(|p| p.qt_valid);
        spec.hidden = obj.hide_in_starmap;
        spec.hierarchy_tag = obj
            .location_hierarchy_tag
            .as_ref()
            .and_then(|t| t.name.clone());
        spec.jurisdiction = obj.jurisdiction.as_ref().and_then(|j| j.name.clone());
        spec.amenities = obj
            .amenities
            .iter()
            .filter_map(|a| {
                let key = a.name.clone()?;
                let display = a.display_name.clone().unwrap_or_else(|| key.clone());
                Some(Amenity { key, display })
            })
            .collect();
        kept.insert(obj.uuid.as_str());
        specs.push((spec, obj.parent_uuid.as_deref()));
    }

    // Entities only present in the positions file (rare). `unknown` ones are internal
    // markers (`UGF_*`, turrets) and are skipped.
    let starmap_ids: HashSet<&str> = starmap.iter().map(|o| o.uuid.as_str()).collect();
    for e in pos.values() {
        if starmap_ids.contains(e.uuid.as_str()) {
            continue;
        }
        let raw_type = e.kind.as_deref().unwrap_or("unknown");
        let Some(name) = e.name.as_deref().filter(|n| !is_placeholder(n)) else {
            continue;
        };
        if raw_type == "unknown" {
            continue;
        }
        let Some(kind) = LocationKind::from_raw(raw_type, name) else {
            continue;
        };
        if kind == LocationKind::Asteroid && !opts.include_asteroid_scatter {
            continue;
        }
        let Some(uuid) = parse_uuid(&e.uuid) else {
            continue;
        };
        let mut spec = NodeSpec::new(uuid, name, kind);
        spec.raw_type = raw_type.to_owned();
        spec.system = e.system.clone();
        spec.position = Some(Vec3::new(e.x, e.y, e.z));
        spec.qt_valid = e.qt_valid;
        spec.hidden = e.hidden;
        kept.insert(e.uuid.as_str());
        specs.push((spec, e.parent_uuid.as_deref()));
    }

    // Raw parent links, to re-parent nodes whose parent was filtered out.
    let mut raw_parent: HashMap<&str, &str> = HashMap::new();
    for o in starmap {
        if let Some(p) = o.parent_uuid.as_deref() {
            raw_parent.insert(o.uuid.as_str(), p);
        }
    }
    for e in &positions.entities {
        if let Some(p) = e.parent_uuid.as_deref() {
            raw_parent.entry(e.uuid.as_str()).or_insert(p);
        }
    }
    // Aliases, resolved against the kept nodes. Curated first so their kind wins on
    // duplicates.
    let mut notes = Vec::new();
    let targets = Targets::new(specs.iter().map(|(s, _)| (s.uuid, s.name.as_str(), s.kind)));
    let mut alias_list = aliases::curated(&targets, &mut notes);
    if let Some(labels) = labels {
        alias_list.extend(aliases::from_labels(labels, &targets));
    }

    let mut b = UniverseBuilder::new();
    for e in &positions.entities {
        if e.kind.as_deref() == Some("Star")
            && let (Some(code), Some(name)) = (e.system.as_deref(), e.name.as_deref())
        {
            b.add_system(code, name);
        }
    }
    for (mut spec, parent) in specs {
        spec.parent = kept_ancestor(&kept, &raw_parent, parent);
        b.add_node(spec);
    }
    for c in &positions.connections {
        if let (Some(a), Some(bb)) = (parse_uuid(&c.entry_uuid), parse_uuid(&c.exit_uuid)) {
            b.add_jump(JumpSpec {
                a,
                b: bb,
                fuel_cost: c.fuel_cost,
            });
        }
    }
    for (target, alias) in alias_list {
        b.add_alias(target, alias);
    }
    (b, notes)
}

/// Closest ancestor (starting at `p`) that was kept as a node.
fn kept_ancestor<'a>(
    kept: &HashSet<&'a str>,
    raw_parent: &HashMap<&'a str, &'a str>,
    mut p: Option<&'a str>,
) -> Option<GameUuid> {
    // Bounded walk: the raw data could contain cycles.
    for _ in 0..64 {
        let u = p?;
        if kept.contains(u) {
            return parse_uuid(u);
        }
        p = raw_parent.get(u).copied();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn placeholders() {
        assert!(is_placeholder("<= UNINITIALIZED =>"));
        assert!(is_placeholder("<= PLACEHOLDER =>"));
        assert!(is_placeholder("@Stanton1_Name"));
        assert!(is_placeholder("  "));
        assert!(!is_placeholder("Lorville"));
    }
}
