//! Travel time between two places for a ship.
//!
//! - Same place: nothing.
//! - Same planet or moon (surface sites, its stations): the pilot's hop time. Surface
//!   coordinates ignore rotation, so a distance there would mean little.
//! - Otherwise [`Universe::route`]: one quantum jump (spool + distance / speed) per
//!   straight leg, plus the stop time for every jump point crossed.
//! - No coordinates or no jump route: a rough guess, flagged, so the route is still ranked.

use sc_universe::{Distance, LocationId, Position, Universe};

use crate::settings::{Settings, Ship};

/// Distance assumed when the universe has none.
const MISSING_M: f64 = 120e9;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LegKind {
    SamePlace,
    /// Both places on or around the same planet or moon: the hop time.
    SameBody,
    Quantum,
    /// Quantum travel through one or more jump points.
    Jump,
    /// No distance known: a rough guess.
    Missing,
}

/// Getting from one stop to the next.
#[derive(Debug, Clone, PartialEq)]
pub struct Leg {
    /// Minutes from leaving one place to arriving at the next (the stop not included).
    pub minutes: f64,
    /// Of which spooling and quantum travel.
    pub quantum_minutes: f64,
    pub kind: LegKind,
    /// Quantum distance flown (jumps add none). None for hops and guesses.
    pub distance: Option<Distance>,
    /// Jump points crossed.
    pub jumps: usize,
    /// A hop time, a guess, or a place whose position is its parent's.
    pub estimated: bool,
}

/// Spool plus cruise for one quantum jump.
pub fn quantum_minutes(ship: &Ship, metres: f64) -> f64 {
    (ship.spool_seconds + metres / (ship.qt_speed_mms.max(1.0) * 1e6)) / 60.0
}

pub fn leg(u: &Universe, ship: &Ship, s: &Settings, a: LocationId, b: LocationId) -> Leg {
    if a == b {
        return Leg {
            minutes: 0.0,
            quantum_minutes: 0.0,
            kind: LegKind::SamePlace,
            distance: Some(Distance::ZERO),
            jumps: 0,
            estimated: false,
        };
    }
    let body = |id| u.nearest_body(id).map(|l| l.id);
    if body(a).is_some() && body(a) == body(b) {
        return Leg {
            minutes: s.hop_minutes,
            quantum_minutes: 0.0,
            kind: LegKind::SameBody,
            distance: None,
            jumps: 0,
            estimated: true,
        };
    }
    match u.route(a, b) {
        Ok(route) => {
            let quantum: f64 = route
                .legs
                .iter()
                .filter_map(|l| match l {
                    sc_universe::Leg::Travel { distance, .. } => {
                        Some(quantum_minutes(ship, distance.metres()))
                    }
                    sc_universe::Leg::Jump { .. } => None,
                })
                .sum();
            let jumps = route.jumps();
            let exact = |id| matches!(u.get(id).position, Position::Exact(_));
            Leg {
                minutes: quantum + jumps as f64 * s.stop_minutes,
                quantum_minutes: quantum,
                kind: if jumps > 0 {
                    LegKind::Jump
                } else {
                    LegKind::Quantum
                },
                distance: Some(route.total),
                jumps,
                estimated: !(exact(a) && exact(b)),
            }
        }
        Err(_) => {
            let quantum = quantum_minutes(ship, MISSING_M);
            Leg {
                minutes: quantum,
                quantum_minutes: quantum,
                kind: LegKind::Missing,
                distance: None,
                jumps: 0,
                estimated: true,
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quantum_time() {
        let ship = Ship {
            qt_speed_mms: 100.0,
            spool_seconds: 6.0,
            ..Ship::default()
        };
        // 6 s spool + 60 Gm at 100 Mm/s (600 s) = 606 s.
        assert!((quantum_minutes(&ship, 60e9) - 10.1).abs() < 1e-9);
    }
}
