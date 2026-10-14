//! Alternative names for starmap locations.
//!
//! The starmap only knows quantum-travel-level places (`Area18`), while in-game text names
//! places inside them (`Riker Memorial Spaceport`). Two sources map those names to a
//! location:
//!
//! - `labels.json` mission location strings of the form `"<venue> in <location>"`
//!   (also `at`), filtered down to unambiguous proper names;
//! - the hand-maintained `data/aliases.json`, for names the game data never ties to a
//!   location (`NB Int. Spaceport`).

use std::collections::{BTreeMap, BTreeSet, HashMap};

use sc_universe::{Alias, AliasKind, GameUuid, LocationKind, normalize};
use serde::Deserialize;

use super::dto::MissionLocations;

pub(crate) const SOURCE_LABELS: &str = "labels";
pub(crate) const SOURCE_CURATED: &str = "curated";

const CURATED_JSON: &str = include_str!("../../data/aliases.json");

/// The nodes aliases may point at, by uuid and by normalised name.
pub(crate) struct Targets<'a> {
    by_name: HashMap<String, Vec<GameUuid>>,
    nodes: HashMap<GameUuid, (&'a str, LocationKind)>,
}

impl<'a> Targets<'a> {
    pub(crate) fn new(nodes: impl IntoIterator<Item = (GameUuid, &'a str, LocationKind)>) -> Self {
        let mut t = Self {
            by_name: HashMap::new(),
            nodes: HashMap::new(),
        };
        for (uuid, name, kind) in nodes {
            t.by_name.entry(normalize(name)).or_default().push(uuid);
            t.nodes.insert(uuid, (name, kind));
        }
        t
    }

    /// The node named `name`, if exactly one has that name.
    fn unique(&self, name: &str) -> Option<GameUuid> {
        match self.by_name.get(&normalize(name))?.as_slice() {
            [one] => Some(*one),
            _ => None,
        }
    }

    fn is_name(&self, name: &str) -> bool {
        self.by_name.contains_key(&normalize(name))
    }
}

#[derive(Debug, Deserialize)]
struct CuratedFile {
    entries: Vec<CuratedEntry>,
}

#[derive(Debug, Deserialize)]
struct CuratedEntry {
    target: String,
    target_name: String,
    #[serde(default)]
    alt_names: Vec<String>,
    #[serde(default)]
    venues: Vec<String>,
}

/// Aliases from the embedded `data/aliases.json`.
///
/// Entries whose target is not in the data (e.g. a partial dataset) are skipped; a target
/// whose name changed is still aliased but reported in `notes`.
pub(crate) fn curated(targets: &Targets<'_>, notes: &mut Vec<String>) -> Vec<(GameUuid, Alias)> {
    let file: CuratedFile = match serde_json::from_str(CURATED_JSON) {
        Ok(f) => f,
        Err(e) => {
            notes.push(format!("curated aliases ignored: {e}"));
            return Vec::new();
        }
    };
    let mut out = Vec::new();
    for entry in file.entries {
        let Ok(uuid) = GameUuid::parse(&entry.target) else {
            notes.push(format!("curated aliases: invalid uuid {:?}", entry.target));
            continue;
        };
        let Some((name, _)) = targets.nodes.get(&uuid) else {
            tracing::debug!("curated alias target {} not in data", entry.target_name);
            continue;
        };
        if normalize(name) != normalize(&entry.target_name) {
            notes.push(format!(
                "curated aliases: {} ({uuid}) is now named {name:?}; check data/aliases.json",
                entry.target_name
            ));
        }
        let alts = entry.alt_names.into_iter().map(|n| (n, AliasKind::AltName));
        let venues = entry.venues.into_iter().map(|n| (n, AliasKind::Venue));
        for (n, kind) in alts.chain(venues) {
            out.push((uuid, Alias::new(n, kind, SOURCE_CURATED)));
        }
    }
    out
}

/// Venue aliases derived from `"<venue> in <location>"` mission location strings.
///
/// Kept only when the venue is a proper name (`Riker Memorial Spaceport`, not `cave` or
/// `a Landing Pad Locker`), is not itself a location name, and every string mentioning it
/// points at the same single, non-body location (a city, station or outpost; "X on
/// Hurston" means somewhere on the planet, not the same place).
pub(crate) fn from_labels(
    labels: &MissionLocations,
    targets: &Targets<'_>,
) -> Vec<(GameUuid, Alias)> {
    // normalised venue -> (display name, targets)
    let mut found: BTreeMap<String, (&str, BTreeSet<GameUuid>)> = BTreeMap::new();
    for text in labels.0.values() {
        let Some((venue, place)) = split_venue(text) else {
            continue;
        };
        if targets.is_name(venue) {
            continue;
        }
        let Some(target) = targets.unique(place) else {
            continue;
        };
        let Some((_, kind)) = targets.nodes.get(&target) else {
            continue;
        };
        if kind.is_body() || matches!(kind, LocationKind::Star | LocationKind::System) {
            continue;
        }
        found
            .entry(normalize(venue))
            .or_insert((venue, BTreeSet::new()))
            .1
            .insert(target);
    }
    found
        .into_values()
        .filter_map(|(venue, set)| match set.len() {
            1 => set
                .first()
                .map(|t| (*t, Alias::new(venue, AliasKind::Venue, SOURCE_LABELS))),
            _ => None,
        })
        .collect()
}

/// Splits `"Riker Memorial Spaceport in Area18"` into `("Riker Memorial Spaceport",
/// "Area18")`. `None` unless the left side looks like a proper name.
fn split_venue(text: &str) -> Option<(&str, &str)> {
    let text = text.trim();
    let i = [" in ", " at "]
        .iter()
        .filter_map(|sep| text.find(sep))
        .min()?;
    let (venue, place) = (text[..i].trim(), text[i + 4..].trim());
    let place = strip_prefix_ci(place, "the ").unwrap_or(place);

    let starts_upper = venue
        .chars()
        .next()
        .is_some_and(|c| c.is_uppercase() || c.is_ascii_digit());
    let article = ["a ", "an ", "the "]
        .iter()
        .any(|a| strip_prefix_ci(venue, a).is_some());
    if !starts_upper || article || venue.contains(" on ") || place.is_empty() {
        return None;
    }
    Some((venue, place))
}

fn strip_prefix_ci<'a>(s: &'a str, prefix: &str) -> Option<&'a str> {
    let head = s.get(..prefix.len())?;
    head.eq_ignore_ascii_case(prefix)
        .then(|| &s[prefix.len()..])
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u128) -> GameUuid {
        GameUuid(uuid::Uuid::from_u128(n))
    }

    #[test]
    fn split_keeps_proper_names_only() {
        assert_eq!(
            split_venue("Riker Memorial Spaceport in Area18"),
            Some(("Riker Memorial Spaceport", "Area18"))
        );
        assert_eq!(
            split_venue("Wallace's Lab in the Grim HEX"),
            Some(("Wallace's Lab", "Grim HEX"))
        );
        assert_eq!(split_venue("cave on Hurston"), None);
        assert_eq!(split_venue("a Landing Pad Locker in Orison"), None);
        assert_eq!(
            split_venue("The Service Entrance in the QV Breaker Station"),
            None
        );
        assert_eq!(
            split_venue("Building A on a Resort Platform in Empyrean Park"),
            None
        );
        assert_eq!(split_venue("Windfall on Pyro III"), None);
        assert_eq!(split_venue("Lorville"), None);
    }

    #[test]
    fn labels_to_aliases() {
        let targets = Targets::new([
            (uuid(1), "Hurston", LocationKind::Planet),
            (uuid(2), "Lorville", LocationKind::City),
            (uuid(3), "Area18", LocationKind::City),
            (uuid(4), "RSI Event Platform", LocationKind::PointOfInterest),
            (uuid(5), "RSI Event Platform", LocationKind::PointOfInterest),
            (uuid(6), "Everus Harbor", LocationKind::Station),
        ]);
        let labels = MissionLocations(
            [
                ("k1", "Riker Memorial Spaceport in Area18"),
                ("k2", "Riker Memorial Spaceport in Area18"),
                ("k3", "Teasa Spaceport in Lorville"),
                ("k4", "Salvage Yard in Hurston"),
                ("k5", "Some Bar in RSI Event Platform"),
                ("k6", "Everus Harbor in Lorville"),
                ("k7", "Twin Bar in Lorville"),
                ("k8", "Twin Bar in Area18"),
            ]
            .into_iter()
            .map(|(k, v)| (k.to_owned(), v.to_owned()))
            .collect(),
        );
        let got: Vec<(GameUuid, String)> = from_labels(&labels, &targets)
            .into_iter()
            .map(|(u, a)| (u, a.name))
            .collect();
        assert_eq!(
            got,
            [
                (uuid(3), "Riker Memorial Spaceport".to_owned()),
                (uuid(2), "Teasa Spaceport".to_owned()),
            ]
        );
    }

    #[test]
    fn curated_file_parses() {
        let file: CuratedFile =
            serde_json::from_str(CURATED_JSON).unwrap_or_else(|e| panic!("{e}"));
        assert!(!file.entries.is_empty());
        for e in &file.entries {
            assert!(GameUuid::parse(&e.target).is_ok(), "{}", e.target);
        }
    }

    #[test]
    fn curated_skips_missing_and_reports_renames() {
        let area18 = GameUuid::parse("c6535844-c615-494e-9dc1-c2d0c6e7190a")
            .unwrap_or_else(|e| panic!("{e}"));
        let targets = Targets::new([(area18, "Area 18 City", LocationKind::City)]);
        let mut notes = Vec::new();
        let got = curated(&targets, &mut notes);
        assert!(got.iter().all(|(u, _)| *u == area18));
        assert!(
            got.iter()
                .any(|(_, a)| a.name == "Riker Memorial Spaceport")
        );
        assert_eq!(notes.len(), 1, "{notes:?}");
    }
}
