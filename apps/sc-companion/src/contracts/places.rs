//! Place names of contracts against the universe: where a name is (shown under its field),
//! places to offer while typing, the closest known place for an unknown name, and the
//! names the player taught the app (`ArcCorp's Yela facility` -> `ArcCorp Mining Area 157`).

use std::collections::BTreeMap;

use sc_contracts::snap::{resolve_place, split_place};
use sc_universe::{LocationId, LocationKind, LookupOptions, Position, Universe, normalize};
use strsim::normalized_levenshtein;

/// Where a place name sits in the universe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceTree {
    /// No name written.
    Empty,
    /// Matched: system first, the place last. `no_distance` when the place has no
    /// coordinates (distances to it are unknown).
    Known {
        path: Vec<String>,
        no_distance: bool,
    },
    /// Several places have this name: each one's path.
    Ambiguous(Vec<Vec<String>>),
    Unknown,
}

/// Lookup of place names, with the remembered names applied.
#[derive(Debug, Clone, Default)]
pub struct Places {
    /// (name, system, lowercase search text: name and aliases), sorted by name.
    index: Vec<(String, String, String)>,
    /// Normalised written name -> known place name.
    remembered: BTreeMap<String, String>,
}

/// Similarity a fuzzy match needs to be offered while typing.
const SUGGEST_MIN: f64 = 0.5;

/// Spelling similarity a known name needs to be offered as a fix for an unknown one.
const FIX_MIN: f64 = 0.6;

impl Places {
    /// `remembered` maps names as written to known place names.
    pub fn new(u: &Universe, remembered: &BTreeMap<String, String>) -> Self {
        let mut index: Vec<(String, String, String)> = u
            .locations()
            .iter()
            .filter(|l| !matches!(l.kind, LocationKind::System))
            .map(|l| {
                let aliases: Vec<&str> = l.aliases.iter().map(|a| a.name.as_str()).collect();
                (
                    l.name.clone(),
                    u.get(l.system).name.clone(),
                    format!("{} {}", l.name, aliases.join(" ")).to_lowercase(),
                )
            })
            .collect();
        index.sort();
        index.dedup_by(|a, b| a.0 == b.0 && a.1 == b.1);
        Self {
            index,
            remembered: remembered
                .iter()
                .map(|(w, p)| (normalize(w), p.clone()))
                .collect(),
        }
    }

    /// The name to look up: the remembered place for a taught name, else the name.
    fn target<'a>(&'a self, name: &'a str) -> &'a str {
        self.remembered
            .get(&normalize(name))
            .map_or(name.trim(), String::as_str)
    }

    pub fn resolve(&self, u: &Universe, name: &str) -> Option<LocationId> {
        if name.trim().is_empty() {
            return None;
        }
        let ids = exact_ids(u, self.target(name));
        match ids.as_slice() {
            [id] => Some(*id),
            [] => resolve_place(u, self.target(name)).map(|m| m.id),
            // Ambiguous: no single answer.
            _ => None,
        }
    }

    pub fn tree(&self, u: &Universe, name: &str) -> PlaceTree {
        if name.trim().is_empty() {
            return PlaceTree::Empty;
        }
        let path = |id| -> Vec<String> {
            u.path_string(id)
                .split(" \u{203a} ")
                .map(str::to_owned)
                .collect()
        };
        let ids = exact_ids(u, self.target(name));
        if ids.len() >= 2 {
            return PlaceTree::Ambiguous(ids.into_iter().map(path).collect());
        }
        match self.resolve(u, name) {
            Some(id) => PlaceTree::Known {
                path: path(id),
                no_distance: matches!(u.get(id).position, Position::Unknown),
            },
            None => PlaceTree::Unknown,
        }
    }

    /// Places for what is typed, as (name, system), best first: names (or aliases) that
    /// contain the text, those starting with it first, then fuzzy matches.
    pub fn suggest(&self, u: &Universe, text: &str, n: usize) -> Vec<(String, String)> {
        let query = text.trim().to_lowercase();
        if query.chars().count() < 2 {
            return Vec::new();
        }
        let mut out: Vec<(String, String)> = self
            .index
            .iter()
            .filter(|p| p.2.contains(&query))
            .map(|p| (p.0.clone(), p.1.clone()))
            .collect();
        out.sort_by_key(|p| !p.0.to_lowercase().starts_with(&query));
        out.truncate(n);
        let (part, _) = split_place(text.trim());
        let opts = LookupOptions {
            limit: n,
            min_score: SUGGEST_MIN,
            ..LookupOptions::default()
        };
        for m in u.find_fuzzy(part, opts) {
            let l = u.get(m.id);
            if out.len() < n && !out.iter().any(|p| p.0 == l.name) {
                out.push((l.name.clone(), u.get(l.system).name.clone()));
            }
        }
        out
    }

    /// The closest known place for a name that doesn't resolve: a name spelled almost the
    /// same (a misread), never one that merely shares a word.
    pub fn fix(&self, u: &Universe, name: &str) -> Option<String> {
        if name.trim().is_empty() || self.resolve(u, name).is_some() {
            return None;
        }
        let (part, _) = split_place(name.trim());
        let want = normalize(part);
        let opts = LookupOptions {
            limit: 5,
            min_score: SUGGEST_MIN,
            ..LookupOptions::default()
        };
        u.find_fuzzy(part, opts)
            .into_iter()
            .map(|m| u.get(m.id).name.clone())
            .map(|known| (normalized_levenshtein(&normalize(&known), &want), known))
            .filter(|(score, known)| *score >= FIX_MIN && normalize(known) != want)
            .max_by(|a, b| a.0.total_cmp(&b.0))
            .map(|(_, known)| known)
    }
}

/// Distinct locations whose name (or alias) is exactly the `<name>` part of `name`.
fn exact_ids(u: &Universe, name: &str) -> Vec<LocationId> {
    let (part, _) = split_place(name.trim());
    let mut ids: Vec<LocationId> = u.find_exact(part).into_iter().map(|m| m.id).collect();
    ids.sort_unstable_by_key(|id| id.index());
    ids.dedup();
    ids
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use sc_data::UniverseSource;

    use super::*;

    fn universe() -> Universe {
        let dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../crates/sc-data/tests/fixtures/scunpacked");
        sc_data::ScunpackedSource::from_dir(dir)
            .load()
            .unwrap_or_else(|e| panic!("{e}"))
            .universe
    }

    fn known_path(t: &PlaceTree) -> Option<String> {
        match t {
            PlaceTree::Known { path, .. } => Some(path.join(" > ")),
            _ => None,
        }
    }

    #[test]
    fn trees_and_suggestions() {
        let u = universe();
        let places = Places::new(&u, &BTreeMap::new());
        assert_eq!(places.tree(&u, " "), PlaceTree::Empty);
        assert_eq!(places.tree(&u, "Nowhere Yard"), PlaceTree::Unknown);
        let lorville = places.tree(&u, "Lorville");
        assert!(
            known_path(&lorville)
                .is_some_and(|p| p.starts_with("Stanton") && p.ends_with("Lorville")),
            "{lorville:?}"
        );
        // `<name> on <parent>`: the name part is looked up.
        assert_eq!(
            known_path(&places.tree(&u, "Lorville on Hurston")),
            known_path(&lorville)
        );
        let s = places.suggest(&u, "lorv", 8);
        assert_eq!(s.first().map(|p| p.0.as_str()), Some("Lorville"));
        assert!(places.suggest(&u, "l", 8).is_empty());
        assert_eq!(
            places.fix(&u, "Lorvile"),
            None,
            "fuzzy matches resolve already"
        );
        assert_eq!(
            places.fix(&u, "Lorville Yard facility"),
            None,
            "shares a word only"
        );
    }

    #[test]
    fn remembered_names_resolve() {
        let u = universe();
        let mut taught = BTreeMap::new();
        taught.insert("Riverside Yard".to_owned(), "Lorville".to_owned());
        let places = Places::new(&u, &taught);
        assert_eq!(
            places.resolve(&u, "riverside  yard"),
            places.resolve(&u, "Lorville")
        );
        assert!(places.resolve(&u, "Lorville").is_some());
    }
}
