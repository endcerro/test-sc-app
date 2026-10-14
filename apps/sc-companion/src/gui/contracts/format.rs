//! Text shown on contract cards (pure, tested; painting is in `mod.rs`).

use sc_contracts::{HaulingContract, truth};

use crate::contracts::Entry;

pub fn title(c: &HaulingContract) -> &str {
    c.title.as_deref().unwrap_or("(title not read)")
}

pub fn reward(c: &HaulingContract) -> String {
    c.reward.map_or_else(
        || "reward ?".to_owned(),
        |r| format!("{} aUEC", truth::thousands(r)),
    )
}

pub fn total_scu(c: &HaulingContract) -> u32 {
    c.cargo.iter().map(|cargo| cargo.scu).sum()
}

/// `Red Wind Linehaul · N/A`: who and until when, the parts that were read.
pub fn byline(c: &HaulingContract) -> String {
    let mut parts = Vec::new();
    if let Some(contractor) = &c.contractor {
        parts.push(contractor.clone());
    }
    if let Some(availability) = &c.availability {
        parts.push(format!("available {availability}"));
    }
    parts.join(" \u{b7} ")
}

/// `168 SCU · max box 16 SCU · 200 rep · 2 screenshots`.
pub fn facts(e: &Entry) -> String {
    let c = &e.contract;
    let mut parts = vec![format!("{} SCU", total_scu(c))];
    if let Some(m) = c.max_box {
        parts.push(format!("max box {m} SCU"));
    }
    if let Some(rep) = c.rep {
        parts.push(format!("{rep} rep"));
    }
    if e.shots.len() > 1 {
        parts.push(format!("{} screenshots", e.shots.len()));
    }
    parts.join(" \u{b7} ")
}

#[cfg(test)]
mod tests {
    use sc_contracts::{Cargo, Place};

    use super::*;

    fn entry() -> Entry {
        let cargo = |scu| Cargo {
            scu,
            commodity: "Gold".into(),
            restricted: false,
            pickups: vec![Place::new("A")],
            dropoff: Place::new("B"),
        };
        Entry {
            id: 1,
            contract: HaulingContract {
                title: None,
                reward: Some(747_000),
                contractor: Some("Red Wind Linehaul".into()),
                availability: Some("N/A".into()),
                max_box: Some(16),
                rep: None,
                cargo: vec![cargo(32), cargo(20)],
            },
            shots: vec![Default::default(), Default::default()],
        }
    }

    #[test]
    fn card_text() {
        let e = entry();
        assert_eq!(title(&e.contract), "(title not read)");
        assert_eq!(reward(&e.contract), "747,000 aUEC");
        assert_eq!(
            byline(&e.contract),
            "Red Wind Linehaul \u{b7} available N/A"
        );
        assert_eq!(
            facts(&e),
            "52 SCU \u{b7} max box 16 SCU \u{b7} 2 screenshots"
        );
        assert_eq!(reward(&HaulingContract::default()), "reward ?");
        assert_eq!(byline(&HaulingContract::default()), "");
    }
}
