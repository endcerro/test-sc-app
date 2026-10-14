//! What the player should double-check on a contract: doubtful OCR reads, inferred or
//! unknown places, missing fields and lines that break the flow rule. Computed live, so
//! edits clear them.

use sc_contracts::{Flow, Place};

use super::Entry;

/// Messages for `entry`, most important first. `known_place` says whether a place name
/// resolves to a location of the universe.
pub fn checks(entry: &Entry, known_place: &mut dyn FnMut(&str) -> bool) -> Vec<String> {
    let c = &entry.contract;
    let mut out = Vec::new();
    if entry.stale {
        out.push(
            "A screenshot was merged into this edited contract: re-parse to include it \
             (discards edits)"
                .to_owned(),
        );
    }
    if !entry.edited {
        out.extend(entry.corrections.iter().cloned());
    }
    if c.title.is_none() {
        out.push("Title not read".to_owned());
    }
    if c.reward.is_none() {
        out.push("Reward not read".to_owned());
    }
    if c.flows.is_empty() {
        out.push("No cargo lines".to_owned());
    }
    let mut unknown: Vec<&str> = Vec::new();
    for (i, f) in c.flows.iter().enumerate() {
        let label = line_label(i, f);
        if f.commodity.trim().is_empty() {
            out.push(format!("{label}: no commodity"));
        }
        if f.pickups.is_empty() {
            out.push(format!("{label}: no pickup"));
        }
        if f.dropoffs.is_empty() {
            out.push(format!("{label}: no drop-off"));
        }
        if !f.dropoffs.is_empty() && f.dropoffs.iter().any(|d| d.scu == 0) {
            out.push(format!("{label}: a drop-off has 0 SCU"));
        }
        if !f.pickups.is_empty() && !f.dropoffs.is_empty() && !f.is_valid() {
            out.push(format!(
                "{label}: several pickups and several drop-offs (a line has one pickup or \
                 one drop-off)"
            ));
        }
        for p in &f.pickups {
            if p.uncertain {
                out.push(format!(
                    "{label}: pickup {} inferred (its Collect line was not on the screenshot)",
                    p.name
                ));
            }
        }
        let places = f.pickups.iter().chain(f.dropoffs.iter().map(|d| &d.place));
        for p in places.filter(|p| is_unknown(p, known_place)) {
            if !unknown.contains(&p.name.as_str()) {
                unknown.push(&p.name);
            }
        }
    }
    out.extend(
        unknown
            .into_iter()
            .map(|name| format!("{name}: not found in the universe data")),
    );
    out
}

/// `Corundum` or `line 2` when the commodity is empty.
fn line_label(i: usize, f: &Flow) -> String {
    if f.commodity.trim().is_empty() {
        format!("Line {}", i + 1)
    } else {
        f.commodity.clone()
    }
}

fn is_unknown(p: &Place, known_place: &mut dyn FnMut(&str) -> bool) -> bool {
    !p.name.trim().is_empty() && !known_place(&p.name)
}

#[cfg(test)]
mod tests {
    use sc_contracts::{Contract, DropOff};

    use super::*;

    fn entry(flows: Vec<Flow>) -> Entry {
        Entry {
            id: 1,
            contract: Contract {
                title: Some("Haul".into()),
                reward: Some(1000),
                flows,
                ..Contract::default()
            },
            shots: Vec::new(),
            corrections: vec!["E'tam commodity, read \"Etam\"".into()],
            edited: false,
            stale: false,
        }
    }

    fn flow(commodity: &str, pickups: &[&str], drops: &[(&str, u32)]) -> Flow {
        Flow {
            commodity: commodity.into(),
            restricted: false,
            pickups: pickups.iter().map(|p| Place::new(*p)).collect(),
            dropoffs: drops
                .iter()
                .map(|(p, scu)| DropOff {
                    place: Place::new(*p),
                    scu: *scu,
                })
                .collect(),
            loaded: false,
        }
    }

    fn known(name: &str) -> bool {
        name != "Nowhere"
    }

    #[test]
    fn clean_contract_only_shows_corrections() {
        let e = entry(vec![flow("Gold", &["A"], &[("B", 4), ("C", 2)])]);
        assert_eq!(checks(&e, &mut known), ["E'tam commodity, read \"Etam\""]);
        let edited = Entry { edited: true, ..e };
        assert!(checks(&edited, &mut known).is_empty());
    }

    #[test]
    fn reports_problems() {
        let mut inferred = flow("Gold", &["A"], &[("B", 1)]);
        inferred.pickups[0].uncertain = true;
        let mut e = entry(vec![
            inferred,
            flow("", &[], &[("Nowhere", 0)]),
            flow("Neon", &["A", "Nowhere"], &[("B", 1), ("C", 1)]),
        ]);
        e.contract.reward = None;
        e.edited = true;
        e.stale = true;
        let got = checks(&e, &mut known);
        assert_eq!(
            got,
            [
                "A screenshot was merged into this edited contract: re-parse to include it \
                 (discards edits)",
                "Reward not read",
                "Gold: pickup A inferred (its Collect line was not on the screenshot)",
                "Line 2: no commodity",
                "Line 2: no pickup",
                "Line 2: a drop-off has 0 SCU",
                "Neon: several pickups and several drop-offs (a line has one pickup or one \
                 drop-off)",
                "Nowhere: not found in the universe data",
            ]
        );
    }
}
