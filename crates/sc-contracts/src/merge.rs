//! Merging objective lists read from several screenshots of a scrolled list.

use strsim::normalized_levenshtein;

use crate::ObjectiveItem;

/// Two items are the same bullet if their normalised text is this similar (OCR noise).
const SAME_ITEM: f64 = 0.85;

/// How many leading items of a later shot may be skipped: half-scrolled bullets at the top
/// edge are faded and often misread.
const MAX_SKIP: usize = 2;

fn same(a: &ObjectiveItem, b: &ObjectiveItem) -> bool {
    std::mem::discriminant(a) == std::mem::discriminant(b)
        && normalized_levenshtein(&a.key(), &b.key()) >= SAME_ITEM
}

/// Merges lists from shots taken top to bottom. Each later list is aligned on the longest
/// run that ends exactly at the end of what is merged so far; its items after that run are
/// appended. Without any overlap the lists are concatenated.
pub fn merge_items(shots: &[Vec<ObjectiveItem>]) -> Vec<ObjectiveItem> {
    let mut merged: Vec<ObjectiveItem> = Vec::new();
    for shot in shots {
        if merged.is_empty() {
            merged.clone_from(shot);
            continue;
        }
        // (run length, start in `shot`)
        let mut best: Option<(usize, usize)> = None;
        for j in 0..shot.len().min(MAX_SKIP + 1) {
            for i in 0..merged.len() {
                let run = merged[i..]
                    .iter()
                    .zip(&shot[j..])
                    .take_while(|(a, b)| same(a, b))
                    .count();
                let reaches_end = i + run == merged.len();
                let contained = j + run == shot.len();
                if run > 0 && (reaches_end || contained) && best.is_none_or(|(r, _)| run > r) {
                    best = Some((run, j));
                }
            }
        }
        match best {
            Some((run, j)) => merged.extend(shot[j + run..].iter().cloned()),
            None => merged.extend(shot.iter().cloned()),
        }
    }
    merged
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(what: &str) -> ObjectiveItem {
        ObjectiveItem::Deliver {
            scu: Some(1),
            commodity: what.into(),
            restricted: false,
            to: "Drop".into(),
        }
    }

    fn c(what: &str) -> ObjectiveItem {
        ObjectiveItem::Collect {
            commodity: what.into(),
            from: "Yela".into(),
        }
    }

    #[test]
    fn merges_scrolled_lists() {
        let a = vec![
            d("Ammo"),
            c("Ammo"),
            d("Hydrogen"),
            c("Hydrogen"),
            d("Medical"),
        ];
        // Second shot starts with a misread half-visible line.
        let b = vec![
            c("Hydragen x"),
            d("Hydrogen"),
            c("Hydrogen"),
            d("Medical"),
            c("Medical"),
            d("Gold"),
        ];
        let m = merge_items(&[a.clone(), b]);
        assert_eq!(m.len(), 7);
        assert_eq!(m[5], c("Medical"));
        assert_eq!(m[6], d("Gold"));
        assert_eq!(merge_items(&[a.clone(), a.clone()]), a);
    }

    #[test]
    fn concatenates_without_overlap() {
        let m = merge_items(&[vec![d("A")], vec![d("Zzzzzz")]]);
        assert_eq!(m.len(), 2);
    }
}
