//! Hauling contracts read from screenshots: OCR ([`reader`]), parsing into flows
//! ([`parse`]), the editable fields of a contract ([`fields`]), where each value was read
//! ([`sources`]), place lookup ([`places`]), what to double-check ([`review`]), contract
//! text import/export ([`board_text`]) and the saved list ([`store`]).
//! Shared by the GUI's Contracts pane and the `contract` CLI command; no egui here.

pub mod board_text;
pub mod fields;
pub mod places;
pub mod reader;
pub mod review;
pub mod sources;
pub mod store;

use std::path::PathBuf;

use sc_contracts::{Contract, Flow, HaulingContract, RegionText};
use sc_ocr::{Region, ScreenLine, ScreenText, text_of};
use sc_universe::Universe;
use serde::{Deserialize, Serialize};

pub use reader::ContractReader;
pub use store::{ContractStore, Entry};

/// Where one OCR line is on its screenshot, in pixels, and how sure the engine was.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct LineBox {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub conf: Option<f32>,
}

/// One box per line of each region's text (`None` when the engine gave no box).
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShotBoxes {
    pub title: Vec<Option<LineBox>>,
    pub reward: Vec<Option<LineBox>>,
    pub details: Vec<Option<LineBox>>,
    pub objectives: Vec<Option<LineBox>>,
}

/// Raw OCR text of one screenshot, kept so a contract can be re-parsed (parser changes,
/// scrolled shots merged later), with the box of every line to show a value's pixels.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ShotText {
    /// Screenshot file the text was read from.
    pub source: Option<PathBuf>,
    pub title: String,
    pub reward: String,
    pub details: String,
    pub objectives: String,
    pub boxes: ShotBoxes,
}

impl ShotText {
    #[cfg_attr(not(feature = "ocr"), allow(dead_code))]
    pub fn from_screen(source: Option<PathBuf>, screen: ScreenText) -> Self {
        let boxes = |lines: &[ScreenLine]| -> Vec<Option<LineBox>> {
            lines
                .iter()
                .map(|l| {
                    l.rect.map(|r| LineBox {
                        x: r.x,
                        y: r.y,
                        w: r.w,
                        h: r.h,
                        conf: l.confidence,
                    })
                })
                .collect()
        };
        Self {
            source,
            title: text_of(&screen.title),
            reward: text_of(&screen.reward),
            details: text_of(&screen.details),
            objectives: text_of(&screen.objectives),
            boxes: ShotBoxes {
                title: boxes(&screen.title),
                reward: boxes(&screen.reward),
                details: boxes(&screen.details),
                objectives: boxes(&screen.objectives),
            },
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

    /// The lines of a region, each with its box when known.
    pub fn lines(&self, region: Region) -> Vec<(&str, Option<LineBox>)> {
        let (text, boxes) = match region {
            Region::Title => (&self.title, &self.boxes.title),
            Region::Reward => (&self.reward, &self.boxes.reward),
            Region::Details => (&self.details, &self.boxes.details),
            Region::Objectives => (&self.objectives, &self.boxes.objectives),
        };
        text.lines()
            .enumerate()
            .map(|(i, l)| (l, boxes.get(i).copied().flatten()))
            .collect()
    }
}

/// What snapping changed: the name as read and the known name it became.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Read {
    pub kind: ReadKind,
    pub read: String,
    pub known: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReadKind {
    Commodity,
    Place,
}

/// A parsed screenshot (or several shots of one contract).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Parsed {
    /// The objectives as listed on screen, names snapped.
    pub objectives: HaulingContract,
    /// The same contract as flows.
    pub contract: Contract,
    /// Names that snapping changed: OCR reads worth a look.
    pub reads: Vec<Read>,
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
        reads: snapped_names(&read, &objectives),
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

/// Every name that snapping changed (each once).
fn snapped_names(read: &HaulingContract, snapped: &HaulingContract) -> Vec<Read> {
    let mut out: Vec<Read> = Vec::new();
    let mut push = |kind, read: &str, known: &str| {
        let r = Read {
            kind,
            read: read.to_owned(),
            known: known.to_owned(),
        };
        if read != known && !out.contains(&r) {
            out.push(r);
        }
    };
    for (r, s) in read.cargo.iter().zip(&snapped.cargo) {
        push(ReadKind::Commodity, &r.commodity, &s.commodity);
        let read_places = r.pickups.iter().chain(std::iter::once(&r.dropoff));
        let snapped_places = s.pickups.iter().chain(std::iter::once(&s.dropoff));
        for (rp, sp) in read_places.zip(snapped_places) {
            push(ReadKind::Place, &rp.name, &sp.name);
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
    fn lines_keep_their_boxes() {
        let b = LineBox {
            x: 1,
            y: 2,
            w: 3,
            h: 4,
            conf: Some(0.9),
        };
        let shot = ShotText {
            reward: "12,000\nN/A".into(),
            boxes: ShotBoxes {
                reward: vec![Some(b)],
                ..ShotBoxes::default()
            },
            ..ShotText::default()
        };
        assert_eq!(
            shot.lines(Region::Reward),
            [("12,000", Some(b)), ("N/A", None)]
        );
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
            objectives: "Deliver 0/8 SCU of Hydragen Fuel to Baijini Point.\n\
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
            parsed.reads,
            [Read {
                kind: ReadKind::Commodity,
                read: "Hydragen Fuel".into(),
                known: "Hydrogen Fuel".into()
            }]
        );
    }
}
