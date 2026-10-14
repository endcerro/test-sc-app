//! A contract as a haul to plan: commodity **flows** between pickups and drop-offs.
//!
//! The screen ([`HaulingContract`]) lists one `Deliver` objective per drop-off, each with the
//! places to collect from. For planning, what matters is where each commodity goes:
//!
//! - one pickup can feed several drop-offs (`Corundum: Wide Forest -> Seraphim 5, Baijini 6`);
//! - several pickups can feed one drop-off (`Aluminum: SAL-5 + SAL-2 -> Baijini 3`); the
//!   game does not say how much comes from each.
//!
//! A [`Flow`] is one of these: one commodity with one pickup, or one drop-off (or both).

use sc_universe::normalize;

use crate::{Cargo, HaulingContract, Place};

/// A contract: the header read from the screen and its commodity flows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Contract {
    pub title: Option<String>,
    /// Reward in aUEC.
    pub reward: Option<u64>,
    pub contractor: Option<String>,
    /// `N/A` or a countdown such as `2h 1m`.
    pub availability: Option<String>,
    /// Largest cargo container the contract uses, in SCU.
    pub max_box: Option<u32>,
    pub rep: Option<u32>,
    pub flows: Vec<Flow>,
}

/// One commodity moving from its pickups to its drop-offs.
///
/// Valid flows have exactly one pickup or exactly one drop-off (see [`Flow::is_valid`]):
/// with several of both, the amounts per route would be unknown.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Flow {
    /// Commodity name without the `[!]` marker.
    pub commodity: String,
    /// The `[!]` marker the game shows before restricted commodities.
    pub restricted: bool,
    pub pickups: Vec<Place>,
    pub dropoffs: Vec<DropOff>,
    /// Set by the player once the cargo is aboard.
    pub loaded: bool,
}

/// Where a part of a flow is delivered, and how much.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DropOff {
    pub place: Place,
    pub scu: u32,
}

impl Flow {
    /// Total SCU (sum of the drop-offs).
    pub fn scu(&self) -> u32 {
        self.dropoffs.iter().map(|d| d.scu).sum()
    }

    /// One pickup, or one drop-off.
    pub fn is_valid(&self) -> bool {
        self.pickups.len() == 1 || self.dropoffs.len() == 1
    }

    /// Commodity as displayed, with the `[!]` marker.
    pub fn display_commodity(&self) -> String {
        if self.restricted {
            format!("[!] {}", self.commodity)
        } else {
            self.commodity.clone()
        }
    }
}

impl Contract {
    pub fn total_scu(&self) -> u32 {
        self.flows.iter().map(Flow::scu).sum()
    }

    /// Groups the screen's `Deliver` objectives into flows: lines of the same commodity from
    /// the same single pickup become one flow with several drop-offs; lines with several
    /// pickups become one flow per drop-off. A place delivered twice in one flow gets the
    /// amounts added. Flows keep the order of their first line. A merged pickup is uncertain
    /// only if it is uncertain on every line.
    pub fn from_objectives(c: &HaulingContract) -> Self {
        let mut flows: Vec<Flow> = Vec::new();
        for cargo in &c.cargo {
            let dropoff = DropOff {
                place: cargo.dropoff.clone(),
                scu: cargo.scu,
            };
            let target = flows.iter_mut().find(|f| {
                same_goods(f, cargo)
                    && same_places(&f.pickups, &cargo.pickups)
                    && (cargo.pickups.len() == 1
                        || f.dropoffs
                            .first()
                            .is_some_and(|d| same_place(&d.place, &dropoff.place)))
            });
            match target {
                Some(flow) => {
                    for (have, new) in flow.pickups.iter_mut().zip(&cargo.pickups) {
                        have.uncertain &= new.uncertain;
                    }
                    match flow
                        .dropoffs
                        .iter_mut()
                        .find(|d| same_place(&d.place, &dropoff.place))
                    {
                        Some(d) => {
                            d.scu += dropoff.scu;
                            d.place.uncertain &= dropoff.place.uncertain;
                        }
                        None => flow.dropoffs.push(dropoff),
                    }
                }
                None => flows.push(Flow {
                    commodity: cargo.commodity.clone(),
                    restricted: cargo.restricted,
                    pickups: cargo.pickups.clone(),
                    dropoffs: vec![dropoff],
                    loaded: false,
                }),
            }
        }
        Self {
            title: c.title.clone(),
            reward: c.reward,
            contractor: c.contractor.clone(),
            availability: c.availability.clone(),
            max_box: c.max_box,
            rep: c.rep,
            flows,
        }
    }

    /// Back to the screen's form: one `Deliver` line per drop-off, with the flow's pickups.
    pub fn to_objectives(&self) -> HaulingContract {
        HaulingContract {
            title: self.title.clone(),
            reward: self.reward,
            contractor: self.contractor.clone(),
            availability: self.availability.clone(),
            max_box: self.max_box,
            rep: self.rep,
            cargo: self
                .flows
                .iter()
                .flat_map(|f| {
                    f.dropoffs.iter().map(|d| Cargo {
                        scu: d.scu,
                        commodity: f.commodity.clone(),
                        restricted: f.restricted,
                        pickups: f.pickups.clone(),
                        dropoff: d.place.clone(),
                    })
                })
                .collect(),
        }
    }
}

fn same_goods(f: &Flow, c: &Cargo) -> bool {
    f.restricted == c.restricted && normalize(&f.commodity) == normalize(&c.commodity)
}

fn same_place(a: &Place, b: &Place) -> bool {
    normalize(&a.name) == normalize(&b.name)
}

/// Same places in the same order (pickup lists come from the screen in a stable order).
fn same_places(a: &[Place], b: &[Place]) -> bool {
    a.len() == b.len() && a.iter().zip(b).all(|(x, y)| same_place(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn line(scu: u32, commodity: &str, pickups: &[&str], dropoff: &str) -> Cargo {
        Cargo {
            scu,
            commodity: commodity.into(),
            restricted: false,
            pickups: pickups.iter().map(|p| Place::new(*p)).collect(),
            dropoff: Place::new(dropoff),
        }
    }

    fn contract(cargo: Vec<Cargo>) -> Contract {
        Contract::from_objectives(&HaulingContract {
            cargo,
            ..HaulingContract::default()
        })
    }

    #[test]
    fn one_pickup_many_dropoffs() {
        let c = contract(vec![
            line(5, "Corundum", &["A"], "B"),
            line(6, "Corundum", &["A"], "C"),
            line(2, "Corundum", &["A"], "B"),
        ]);
        assert_eq!(c.flows.len(), 1);
        let f = &c.flows[0];
        assert_eq!(f.scu(), 13);
        let drops: Vec<(&str, u32)> = f
            .dropoffs
            .iter()
            .map(|d| (d.place.name.as_str(), d.scu))
            .collect();
        assert_eq!(drops, [("B", 7), ("C", 6)]);
        assert!(f.is_valid());
    }

    #[test]
    fn many_pickups_stay_one_flow_per_dropoff() {
        let c = contract(vec![
            line(3, "Aluminum", &["A", "B"], "X"),
            line(4, "Aluminum", &["A", "B"], "Y"),
            line(1, "Aluminum", &["A", "B"], "X"),
        ]);
        assert_eq!(c.flows.len(), 2);
        assert_eq!(c.flows[0].scu(), 4);
        assert_eq!(c.flows[1].scu(), 4);
        assert!(c.flows.iter().all(Flow::is_valid));
    }

    #[test]
    fn different_goods_or_pickups_do_not_merge() {
        let mut restricted = line(1, "Neon", &["A"], "B");
        restricted.restricted = true;
        let c = contract(vec![
            line(1, "Neon", &["A"], "B"),
            restricted,
            line(1, "Neon", &["Z"], "B"),
            line(1, "Gold", &["A"], "B"),
        ]);
        assert_eq!(c.flows.len(), 4);
    }

    #[test]
    fn certain_pickup_wins() {
        let mut inferred = line(20, "Medical Supplies", &["Yela"], "C");
        inferred.pickups[0].uncertain = true;
        let c = contract(vec![line(20, "Medical Supplies", &["Yela"], "B"), inferred]);
        assert_eq!(c.flows.len(), 1);
        assert!(!c.flows[0].pickups[0].uncertain);
    }

    #[test]
    fn invalid_when_many_both_sides() {
        let f = Flow {
            pickups: vec![Place::new("A"), Place::new("B")],
            dropoffs: vec![
                DropOff {
                    place: Place::new("X"),
                    scu: 1,
                },
                DropOff {
                    place: Place::new("Y"),
                    scu: 1,
                },
            ],
            ..Flow::default()
        };
        assert!(!f.is_valid());
    }
}
