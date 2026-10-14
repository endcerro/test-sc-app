//! Snapping OCR'd names to known names: commodities from the embedded list, places from
//! the [`Universe`] (names and aliases).

use std::sync::OnceLock;

use sc_universe::{LookupOptions, Universe, normalize};
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

/// Place names are `<name> <link> <parent>` (`Ruin Station above Pyro VI`). Only `<name>`
/// is snapped: contracts call planets by designation (`Pyro VI`) while the starmap uses
/// their names (`Terminus`), so parents and descriptive names (`Yela's asteroid belt`,
/// `ArcCorp's Yela facility`) have no reliable counterpart and are kept as read.
pub fn place(universe: &Universe, name: &str) -> String {
    let opts = LookupOptions {
        limit: 1,
        ..LookupOptions::default()
    };
    let snap = |part: &str| -> String {
        if !universe.find_exact(part).is_empty() {
            return part.to_owned();
        }
        universe
            .find_fuzzy(part, opts)
            .first()
            .map(|m| universe.matched_name(m))
            .filter(|known| {
                normalized_levenshtein(&normalize(known), &normalize(part)) >= PLACE_MIN
            })
            .map_or_else(|| part.to_owned(), str::to_owned)
    };
    let words: Vec<&str> = name.split(' ').collect();
    match words
        .iter()
        .position(|w| ["above", "on", "in", "within"].contains(w))
    {
        Some(i) if i > 0 && i + 1 < words.len() => {
            format!("{} {}", snap(&words[..i].join(" ")), words[i..].join(" "))
        }
        _ => snap(name),
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
    fn snaps_commodities() {
        assert_eq!(commodity("Hydragen Fuel"), Some("Hydrogen Fuel"));
        assert_eq!(commodity("Corundum (Raw)"), Some("Corundum (Raw)"));
        assert_eq!(commodity("Pressurlzed lce"), Some("Pressurized Ice"));
        assert_eq!(commodity("WIDOW"), Some("WiDoW"));
        assert_eq!(commodity("qqqqqq"), None);
    }
}
