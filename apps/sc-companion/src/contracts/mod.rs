//! Hauling contracts read from screenshots: OCR ([`reader`]), parsing into flows
//! ([`parse`]), what to double-check ([`checks`]) and the saved list ([`store`]).
//! Shared by the GUI's Contracts pane and the `contract` CLI command; no egui here.

pub mod checks;
pub mod reader;
pub mod store;

use std::path::PathBuf;

use sc_contracts::{Contract, Flow, HaulingContract, RegionText};
use sc_ocr::ScreenText;
use sc_universe::Universe;
use serde::{Deserialize, Serialize};

pub use reader::ContractReader;
pub use store::{ContractStore, Entry};

/// Raw OCR text of one screenshot, kept so a contract can be re-parsed (parser changes,
/// scrolled shots merged later).
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShotText {
    /// Screenshot file the text was read from.
    pub source: Option<PathBuf>,
    pub title: String,
    pub reward: String,
    pub details: String,
    pub objectives: String,
}

impl ShotText {
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    pub fn from_screen(source: Option<PathBuf>, screen: ScreenText) -> Self {
        Self {
            source,
            title: screen.title,
            reward: screen.reward,
            details: screen.details,
            objectives: screen.objectives,
        }
    }

    /// Parser input; empty regions are `None` so a later shot of the same contract can
    /// supply them.
    pub fn region_text(&self) -> RegionText {
        let some = |s: &str| (!s.trim().is_empty()).then(|| s.to_owned());
        RegionText {
            title: some(&self.title),
            reward: some(&self.reward),
            details: some(&self.details),
            objectives: self.objectives.lines().map(str::to_owned).collect(),
        }
    }
}

/// A parsed screenshot (or several shots of one contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The objectives as listed on screen, names snapped.
    pub objectives: HaulingContract,
    /// The same contract as flows.
    pub contract: Contract,
    /// Names that snapping changed: OCR reads worth a look (`E'tam commodity, read "Etam"`).
    pub corrections: Vec<String>,
}

/// Parses one or more shots of the same contract, snaps commodity and place names to the
/// known ones and groups the objectives into flows.
pub fn parse(shots: &[ShotText], universe: &Universe) -> Parsed {
    let regions: Vec<RegionText> = shots.iter().map(ShotText::region_text).collect();
    let read = sc_contracts::parse::contract_from_shots(&regions);
    let mut objectives = read.clone();
    sc_contracts::snap::contract(&mut objectives, Some(universe));
    Parsed {
        contract: Contract::from_objectives(&objectives),
        corrections: corrections(&read, &objectives),
        objectives,
    }
}

/// `11 SCU Corundum: Wide Forest -> Seraphim (5), Baijini (6)`; drop-off amounts are shown
/// when there are several. Inferred places keep their `?`.
pub fn flow_line(f: &Flow) -> String {
    let pickups: Vec<String> = f.pickups.iter().map(ToString::to_string).collect();
    let drops: Vec<String> = f
        .dropoffs
        .iter()
        .map(|d| {
            if f.dropoffs.len() > 1 {
                format!("{} ({})", d.place, d.scu)
            } else {
                d.place.to_string()
            }
        })
        .collect();
    format!(
        "{} SCU {}: {} -> {}",
        f.scu(),
        f.display_commodity(),
        pickups.join(" + "),
        drops.join(", ")
    )
}

/// One message per name that snapping changed (each name once).
fn corrections(read: &HaulingContract, snapped: &HaulingContract) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut push = |what: &str, read: &str, known: &str| {
        let msg = format!("{known} {what}, read \"{read}\"");
        if read != known && !out.contains(&msg) {
            out.push(msg);
        }
    };
    for (r, s) in read.cargo.iter().zip(&snapped.cargo) {
        push("commodity", &r.commodity, &s.commodity);
        let read_places = r.pickups.iter().chain(std::iter::once(&r.dropoff));
        let snapped_places = s.pickups.iter().chain(std::iter::once(&s.dropoff));
        for (rp, sp) in read_places.zip(snapped_places) {
            push("place", &rp.name, &sp.name);
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_regions_are_none() {
        let shot = ShotText {
            title: "  \n".into(),
            reward: "12,000".into(),
            objectives: "Deliver 0/8 SCU of Gold to A.\nCollect Gold from B.".into(),
            ..ShotText::default()
        };
        let r = shot.region_text();
        assert_eq!(r.title, None);
        assert_eq!(r.reward.as_deref(), Some("12,000"));
        assert_eq!(r.details, None);
        assert_eq!(r.objectives.len(), 2);
    }

    #[test]
    fn flow_lines() {
        use sc_contracts::{DropOff, Place};
        let drop = |name: &str, scu| DropOff {
            place: Place::new(name),
            scu,
        };
        let mut f = Flow {
            commodity: "Corundum".into(),
            restricted: false,
            pickups: vec![Place::new("A")],
            dropoffs: vec![drop("B", 5), drop("C", 6)],
            loaded: false,
        };
        assert_eq!(flow_line(&f), "11 SCU Corundum: A -> B (5), C (6)");
        f.pickups = vec![Place::new("A"), Place::uncertain("Z")];
        f.dropoffs.truncate(1);
        f.restricted = true;
        assert_eq!(flow_line(&f), "5 SCU [!] Corundum: A + Z? -> B");
    }

    #[test]
    fn snapped_names_are_reported() {
        let shot = ShotText {
            title: "Haul".into(),
            reward: "12,000".into(),
            objectives: "Deliver 0/8 SCU of Hydragen Fuel to Baijini Point.
                         Collect Hydragen Fuel from Port Tressler."
                .into(),
            ..ShotText::default()
        };
        let u = sc_universe::UniverseBuilder::new()
            .build(sc_universe::DataMeta::default())
            .0;
        let parsed = parse(&[shot], &u);
        assert_eq!(parsed.contract.flows.len(), 1);
        assert_eq!(parsed.contract.flows[0].commodity, "Hydrogen Fuel");
        assert_eq!(
            parsed.corrections,
            ["Hydrogen Fuel commodity, read \"Hydragen Fuel\""]
        );
    }
}
