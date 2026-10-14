//! The editable form of a contract: every field as the text in its box, so typing never
//! fights the parser. Converted back to a [`Contract`] when an edit is committed.

use sc_contracts::{Contract, DropOff, Flow, Place};

use super::format;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Draft {
    /// The contract this draft was made from (to notice outside changes).
    pub base: Contract,
    pub title: String,
    pub reward: String,
    pub contractor: String,
    pub availability: String,
    pub max_box: String,
    pub rep: String,
    pub flows: Vec<FlowDraft>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FlowDraft {
    pub commodity: String,
    pub restricted: bool,
    pub loaded: bool,
    pub pickups: Vec<PlaceDraft>,
    pub dropoffs: Vec<PlaceDraft>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct PlaceDraft {
    pub name: String,
    /// Inferred, not read; cleared when the player edits the name.
    pub uncertain: bool,
    /// Drop-offs only.
    pub scu: String,
}

impl FlowDraft {
    /// A new line: one empty pickup and one empty drop-off.
    pub fn empty() -> Self {
        Self {
            pickups: vec![PlaceDraft::default()],
            dropoffs: vec![PlaceDraft::default()],
            ..Self::default()
        }
    }

    /// Total of the drop-off amounts typed so far.
    pub fn scu(&self) -> u32 {
        self.dropoffs
            .iter()
            .filter_map(|d| format::parse_number::<u32>(&d.scu))
            .sum()
    }

    /// A pickup can be added while there is at most one drop-off, and the other way round.
    pub fn can_add_pickup(&self) -> bool {
        self.dropoffs.len() <= 1
    }

    pub fn can_add_dropoff(&self) -> bool {
        self.pickups.len() <= 1
    }
}

impl Draft {
    pub fn new(c: &Contract) -> Self {
        let text = |s: &Option<String>| s.clone().unwrap_or_default();
        let number = |n: Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
        let place = |p: &Place| PlaceDraft {
            name: p.name.clone(),
            uncertain: p.uncertain,
            scu: String::new(),
        };
        Self {
            base: c.clone(),
            title: text(&c.title),
            reward: format::reward(c.reward),
            contractor: text(&c.contractor),
            availability: text(&c.availability),
            max_box: number(c.max_box),
            rep: number(c.rep),
            flows: c
                .flows
                .iter()
                .map(|f| FlowDraft {
                    commodity: f.commodity.clone(),
                    restricted: f.restricted,
                    loaded: f.loaded,
                    pickups: f.pickups.iter().map(place).collect(),
                    dropoffs: f
                        .dropoffs
                        .iter()
                        .map(|d| PlaceDraft {
                            scu: d.scu.to_string(),
                            ..place(&d.place)
                        })
                        .collect(),
                })
                .collect(),
        }
    }

    /// The contract as typed: blank text is "not read", numbers keep their digits, blank
    /// places are dropped.
    pub fn to_contract(&self) -> Contract {
        let text = |s: &str| {
            let s = s.trim();
            (!s.is_empty()).then(|| s.to_owned())
        };
        let place = |p: &PlaceDraft| Place {
            name: p.name.trim().to_owned(),
            uncertain: p.uncertain,
        };
        let filled = |p: &&PlaceDraft| !p.name.trim().is_empty();
        Contract {
            title: text(&self.title),
            reward: format::parse_number(&self.reward),
            contractor: text(&self.contractor),
            availability: text(&self.availability),
            max_box: format::parse_number(&self.max_box),
            rep: format::parse_number(&self.rep),
            flows: self
                .flows
                .iter()
                .map(|f| Flow {
                    commodity: f.commodity.trim().to_owned(),
                    restricted: f.restricted,
                    pickups: f.pickups.iter().filter(filled).map(place).collect(),
                    dropoffs: f
                        .dropoffs
                        .iter()
                        .filter(|d| filled(d) || !d.scu.trim().is_empty())
                        .map(|d| DropOff {
                            place: place(d),
                            scu: format::parse_number(&d.scu).unwrap_or(0),
                        })
                        .collect(),
                    loaded: f.loaded,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn contract() -> Contract {
        Contract {
            title: Some("Haul".into()),
            reward: Some(525_500),
            contractor: Some("Dead Saints".into()),
            availability: None,
            max_box: Some(8),
            rep: None,
            flows: vec![Flow {
                commodity: "Neon".into(),
                restricted: true,
                pickups: vec![Place::uncertain("A")],
                dropoffs: vec![
                    DropOff {
                        place: Place::new("B"),
                        scu: 5,
                    },
                    DropOff {
                        place: Place::new("C"),
                        scu: 3,
                    },
                ],
                loaded: true,
            }],
        }
    }

    #[test]
    fn round_trip() {
        let c = contract();
        let d = Draft::new(&c);
        assert_eq!(d.reward, "525,500");
        assert_eq!(d.flows[0].scu(), 8);
        assert_eq!(d.to_contract(), c);
    }

    #[test]
    fn typed_values() {
        let mut d = Draft::new(&contract());
        d.title = "  ".into();
        d.reward = "1 000 aUEC".into();
        d.max_box = "x".into();
        d.flows[0].dropoffs[1].scu = "".into();
        d.flows[0].pickups.push(PlaceDraft::default());
        d.flows.push(FlowDraft::empty());
        let c = d.to_contract();
        assert_eq!(c.title, None);
        assert_eq!(c.reward, Some(1000));
        assert_eq!(c.max_box, None);
        assert_eq!(c.flows[0].dropoffs[1].scu, 0);
        assert_eq!(c.flows[0].pickups.len(), 1, "blank place dropped");
        assert_eq!(c.flows[1].pickups.len(), 0);
        assert_eq!(c.flows[1].dropoffs.len(), 0);
    }

    #[test]
    fn one_pickup_or_one_dropoff() {
        let d = Draft::new(&contract());
        let f = &d.flows[0];
        assert!(f.can_add_dropoff());
        assert!(!f.can_add_pickup());
        assert!(FlowDraft::empty().can_add_pickup());
    }
}
