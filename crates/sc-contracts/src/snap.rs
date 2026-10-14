//! Snapping OCR'd names to known names: commodities from the embedded list, places from
//! the [`Universe`] (names and aliases).

use std::sync::OnceLock;

use sc_universe::{LookupOptions, Match, Universe, normalize};
use strsim::normalized_levenshtein;

use crate::HaulingContract;

/// Commodity names from scunpacked-data `resources/commodities.json` (game 4.10.2),
/// placeholders removed. Regenerate when the game adds commodities; UEX will replace it.
const COMMODITIES_JSON: &str = include_str!("../data/commodities.json");

/// Minimum similarity to replace a commodity name.
const COMMODITY_MIN: f64 = 0.75;

pub fn commodities() -> &'static [String] {
    static LIST: OnceLock<Vec<String>> = OnceLock::new();
    LIST.get_or_init(|| serde_json::from_str(COMMODITIES_JSON).unwrap_or_default())
}

/// Closest known commodity, if similar enough.
pub fn commodity(name: &str) -> Option<&'static str> {
    let want = normalize(name);
    commodities()
        .iter()
        .map(|c| (c, normalized_levenshtein(&normalize(c), &want)))
        .filter(|(_, s)| *s >= COMMODITY_MIN)
        .max_by(|a, b| a.1.total_cmp(&b.1))
        .map(|(c, _)| c.as_str())
}

/// Minimum edit similarity between the OCR'd and the known name to replace it: only fixes
/// spelling, never swaps a name for a different, merely similar-looking one.
const PLACE_MIN: f64 = 0.85;

/// Words joining a place name to its parent (`Ruin Station above Pyro VI`).
const LINKS: [&str; 4] = ["above", "on", "in", "within"];

/// `Ruin Station above Pyro VI` -> (`Ruin Station`, `Some("above Pyro VI")`). The link must
/// follow at least one word and be followed by one.
pub fn split_place(name: &str) -> (&str, Option<&str>) {
    let mut offset = 0;
    for (i, word) in name.split(' ').enumerate() {
        if i > 0 && LINKS.contains(&word) && offset + word.len() < name.len() {
            return (&name[..offset - 1], Some(&name[offset..]));
        }
        offset += word.len() + 1;
    }
    (name, None)
}

/// The location a contract place name refers to: its `<name>` part (see [`split_place`])
/// matched exactly, or fuzzily when the spelling is close enough.
pub fn resolve_place(universe: &Universe, name: &str) -> Option<Match> {
    let (part, _) = split_place(name.trim());
    if let Some(m) = universe.find_exact(part).into_iter().next() {
        return Some(m);
    }
    let opts = LookupOptions {
        limit: 1,
        ..LookupOptions::default()
    };
    universe
        .find_fuzzy(part, opts)
        .into_iter()
        .next()
        .filter(|m| {
            let known = universe.matched_name(m);
            normalized_levenshtein(&normalize(known), &normalize(part)) >= PLACE_MIN
        })
}

/// Place names are `<name> <link> <parent>` (`Ruin Station above Pyro VI`). Only `<name>`
/// is snapped: contracts call planets by designation (`Pyro VI`) while the starmap uses
/// their names (`Terminus`), so parents and descriptive names (`Yela's asteroid belt`,
/// `ArcCorp's Yela facility`) have no reliable counterpart and are kept as read.
pub fn place(universe: &Universe, name: &str) -> String {
    let (part, rest) = split_place(name);
    let snapped = if universe.find_exact(part).is_empty() {
        resolve_place(universe, part).map_or(part, |m| universe.matched_name(&m))
    } else {
        part
    };
    match rest {
        Some(rest) => format!("{snapped} {rest}"),
        None => snapped.to_owned(),
    }
}

/// Snaps every commodity, and every place when a universe is given.
pub fn contract(c: &mut HaulingContract, universe: Option<&Universe>) {
    for cargo in &mut c.cargo {
        if let Some(known) = commodity(&cargo.commodity) {
            known.clone_into(&mut cargo.commodity);
        }
        if let Some(u) = universe {
            for p in cargo
                .pickups
                .iter_mut()
                .chain(std::iter::once(&mut cargo.dropoff))
            {
                if !p.name.is_empty() {
                    p.name = place(u, &p.name);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_is_embedded() {
        assert!(commodities().len() > 150);
        assert!(commodities().iter().any(|c| c == "E'tam"));
    }

    #[test]
    fn splits_place_names() {
        assert_eq!(
            split_place("Ruin Station above Pyro VI"),
            ("Ruin Station", Some("above Pyro VI"))
        );
        assert_eq!(
            split_place("Sunset Mesa on Pyro II"),
            ("Sunset Mesa", Some("on Pyro II"))
        );
        assert_eq!(split_place("Baijini Point"), ("Baijini Point", None));
        assert_eq!(split_place("on Pyro II"), ("on Pyro II", None));
        assert_eq!(split_place("Station in"), ("Station in", None));
    }

    #[test]
    fn snaps_commodities() {
        assert_eq!(commodity("Hydragen Fuel"), Some("Hydrogen Fuel"));
        assert_eq!(commodity("Corundum (Raw)"), Some("Corundum (Raw)"));
        assert_eq!(commodity("Pressurlzed lce"), Some("Pressurized Ice"));
        assert_eq!(commodity("WIDOW"), Some("WiDoW"));
        assert_eq!(commodity("qqqqqq"), None);
    }
}
