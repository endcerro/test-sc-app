//! Name normalisation and lookup, tuned for matching OCR output to location names.

use std::collections::HashMap;

use unicode_normalization::UnicodeNormalization;
use unicode_normalization::char::is_combining_mark;

use crate::LocationId;

/// Normalises a location name for comparison.
///
/// - Unicode NFKD, combining marks dropped (`é` -> `e`)
/// - lowercase
/// - apostrophes removed (`Rod's` -> `rods`), `&` -> `and`
/// - every other non-alphanumeric char becomes a space, whitespace collapsed
/// - British/US spelling folded (`centre` -> `center`)
pub fn normalize(name: &str) -> String {
    let mut cleaned = String::with_capacity(name.len());
    for c in name.nfkd().filter(|c| !is_combining_mark(*c)) {
        match c {
            '\'' | '\u{2019}' | '\u{2018}' | '`' => {}
            '&' => cleaned.push_str(" and "),
            c if c.is_alphanumeric() => cleaned.extend(c.to_lowercase()),
            _ => cleaned.push(' '),
        }
    }
    let tokens: Vec<&str> = cleaned
        .split_whitespace()
        .map(|t| match t {
            "centre" => "center",
            other => other,
        })
        .collect();
    tokens.join(" ")
}

/// Options for [`crate::Universe::find_fuzzy`].
#[derive(Debug, Clone, Copy)]
pub struct LookupOptions {
    /// Only return locations inside this subtree (inclusive).
    pub scope: Option<LocationId>,
    /// Maximum number of matches returned.
    pub limit: usize,
    /// Minimum similarity in `0.0..=1.0`.
    pub min_score: f64,
}

impl Default for LookupOptions {
    fn default() -> Self {
        Self {
            scope: None,
            limit: 5,
            min_score: 0.80,
        }
    }
}

/// A lookup result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub id: LocationId,
    /// Similarity in `0.0..=1.0`; `1.0` means the normalised names are equal.
    pub score: f64,
    /// Index into the location's [`crate::Location::aliases`] when the match came through
    /// an alias rather than the location's own name.
    pub alias: Option<u16>,
}

/// One name pointing at a location: its own name (`alias == None`) or one of its aliases.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Entry {
    id: LocationId,
    alias: Option<u16>,
}

/// Index from normalised names (own names and aliases) to locations. Several locations may
/// share a name.
#[derive(Debug, Clone, Default)]
pub(crate) struct NameIndex {
    exact: HashMap<String, Vec<Entry>>,
    /// Normalised name with spaces removed -> normalised names, so `Area 18` finds `Area18`.
    compact: HashMap<String, Vec<String>>,
}

/// Normalised name with spaces removed.
fn compact(normalized: &str) -> String {
    normalized.chars().filter(|c| *c != ' ').collect()
}

impl NameIndex {
    pub(crate) fn insert(&mut self, name: &str, id: LocationId, alias: Option<u16>) {
        let key = normalize(name);
        if key.is_empty() {
            return;
        }
        let entries = self.exact.entry(key.clone()).or_default();
        if entries.is_empty() {
            self.compact.entry(compact(&key)).or_default().push(key);
        }
        entries.push(Entry { id, alias });
    }

    /// Exact matches on the normalised name, falling back to the space-insensitive form.
    /// Own-name matches come before alias matches; each location appears once.
    pub(crate) fn exact(&self, query: &str) -> Vec<Match> {
        let q = normalize(query);
        let entries: Vec<Entry> = match self.exact.get(&q) {
            Some(e) => e.clone(),
            None => self
                .compact
                .get(&compact(&q))
                .into_iter()
                .flatten()
                .filter_map(|k| self.exact.get(k))
                .flatten()
                .copied()
                .collect(),
        };
        let mut out: Vec<Match> = entries
            .into_iter()
            .map(|e| Match {
                id: e.id,
                score: 1.0,
                alias: e.alias,
            })
            .collect();
        rank_and_dedup(&mut out);
        out
    }

    /// Scores every distinct name against `query`. `accept` filters candidate ids.
    pub(crate) fn fuzzy(
        &self,
        query: &str,
        opts: LookupOptions,
        mut accept: impl FnMut(LocationId) -> bool,
    ) -> Vec<Match> {
        let q = normalize(query);
        if q.is_empty() {
            return Vec::new();
        }
        let qc = compact(&q);
        let mut out: Vec<Match> = Vec::new();
        for (name, entries) in &self.exact {
            let score = if compact(name) == qc {
                1.0
            } else {
                similarity(&q, name)
            };
            if score < opts.min_score {
                continue;
            }
            out.extend(entries.iter().filter(|e| accept(e.id)).map(|e| Match {
                id: e.id,
                score,
                alias: e.alias,
            }));
        }
        rank_and_dedup(&mut out);
        out.truncate(opts.limit);
        out
    }
}

/// Best first: score, then own name before alias, then id. Keeps one match per location.
fn rank_and_dedup(matches: &mut Vec<Match>) {
    matches.sort_by(|a, b| {
        b.score
            .total_cmp(&a.score)
            .then(a.alias.is_some().cmp(&b.alias.is_some()))
            .then(a.id.cmp(&b.id))
            .then(a.alias.cmp(&b.alias))
    });
    let mut seen = std::collections::HashSet::new();
    matches.retain(|m| seen.insert(m.id));
}

/// Similarity of two normalised names.
///
/// Takes the best of Jaro-Winkler (forgiving on suffix noise / truncation) and normalised
/// Levenshtein (forgiving on OCR character substitutions).
fn similarity(a: &str, b: &str) -> f64 {
    if a == b {
        return 1.0;
    }
    strsim::jaro_winkler(a, b).max(strsim::normalized_levenshtein(a, b))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_basics() {
        assert_eq!(
            normalize("  Rod's Fuel 'N Supplies "),
            "rods fuel n supplies"
        );
        assert_eq!(
            normalize("HUR-L1 Green Glade Station"),
            "hur l1 green glade station"
        );
        assert_eq!(
            normalize("Covalex Distribution Centre S1DC06"),
            normalize("covalex distribution center s1dc06")
        );
        assert_eq!(normalize("Dudley & Daughters"), "dudley and daughters");
        assert_eq!(normalize("Café"), "cafe");
    }

    #[test]
    fn compact_key_ignores_spaces() {
        let mut idx = NameIndex::default();
        let a = LocationId::from_index(1);
        let b = LocationId::from_index(2);
        idx.insert("Area18", a, None);
        idx.insert("Grim HEX", b, None);
        assert_eq!(idx.exact("Area 18").first().map(|m| m.id), Some(a));
        assert_eq!(idx.exact("GrimHEX").first().map(|m| m.id), Some(b));
        let fuzzy = idx.fuzzy("area 18", LookupOptions::default(), |_| true);
        assert_eq!(fuzzy.first().map(|m| (m.id, m.score)), Some((a, 1.0)));
    }

    #[test]
    fn own_name_beats_alias() {
        let mut idx = NameIndex::default();
        let own = LocationId::from_index(5);
        let other = LocationId::from_index(1);
        idx.insert("Teasa Spaceport", other, Some(0));
        idx.insert("Teasa Spaceport", own, None);
        idx.insert("Teasa Spaceport", own, Some(3));
        let m = idx.exact("teasa spaceport");
        assert_eq!(m.len(), 2, "one match per location");
        assert_eq!((m[0].id, m[0].alias), (own, None));
        assert_eq!((m[1].id, m[1].alias), (other, Some(0)));
    }

    #[test]
    fn similarity_tolerates_ocr_noise() {
        assert!(similarity("lorvile", "lorville") > 0.9);
        assert!(similarity("ar1al", "arial") > 0.8);
        assert!(similarity("lorville", "orison") < 0.7);
    }
}
