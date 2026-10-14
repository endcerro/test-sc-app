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

/// A fuzzy lookup result.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Match {
    pub id: LocationId,
    /// Similarity in `0.0..=1.0`; `1.0` means the normalised names are equal.
    pub score: f64,
}

/// Index from normalised names to locations. Several locations may share a name.
#[derive(Debug, Clone, Default)]
pub(crate) struct NameIndex {
    exact: HashMap<String, Vec<LocationId>>,
}

impl NameIndex {
    pub(crate) fn insert(&mut self, name: &str, id: LocationId) {
        self.exact.entry(normalize(name)).or_default().push(id);
    }

    pub(crate) fn exact(&self, query: &str) -> &[LocationId] {
        self.exact
            .get(&normalize(query))
            .map(Vec::as_slice)
            .unwrap_or_default()
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
        let mut out: Vec<Match> = Vec::new();
        for (name, ids) in &self.exact {
            let score = similarity(&q, name);
            if score < opts.min_score {
                continue;
            }
            out.extend(
                ids.iter()
                    .copied()
                    .filter(|id| accept(*id))
                    .map(|id| Match { id, score }),
            );
        }
        out.sort_by(|a, b| b.score.total_cmp(&a.score).then(a.id.cmp(&b.id)));
        out.truncate(opts.limit);
        out
    }
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
    fn similarity_tolerates_ocr_noise() {
        assert!(similarity("lorvile", "lorville") > 0.9);
        assert!(similarity("ar1al", "arial") > 0.8);
        assert!(similarity("lorville", "orison") < 0.7);
    }
}
