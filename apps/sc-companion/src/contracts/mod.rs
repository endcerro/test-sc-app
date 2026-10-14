//! Hauling contracts read from screenshots: OCR ([`reader`]) and the saved list ([`store`]).
//! Shared by the GUI's Contracts pane and the `contract` CLI command; no egui here.

pub mod reader;
pub mod store;

use std::path::PathBuf;

use sc_contracts::{HaulingContract, RegionText};
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

/// Parses one or more shots of the same contract and snaps commodity and place names.
pub fn parse(shots: &[ShotText], universe: &Universe) -> HaulingContract {
    let regions: Vec<RegionText> = shots.iter().map(ShotText::region_text).collect();
    let mut contract = sc_contracts::parse::contract_from_shots(&regions);
    sc_contracts::snap::contract(&mut contract, Some(universe));
    contract
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
}
