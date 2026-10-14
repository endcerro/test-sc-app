//! What the pilot sets: the ship, where they are, and what a good route means to them.
//! Saved by the app, so every field has a default (`#[serde(default)]`).

use serde::{Deserialize, Serialize};

use crate::select::Goal;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Ship {
    pub name: String,
    /// Estimated cargo capacity (SCU), for the comfort indicator. None = unknown.
    pub capacity_estimate: Option<f64>,
    /// Absolute ceiling (SCU). Routes above it are never proposed.
    pub hard_max: Option<f64>,
    /// Largest container (SCU) the cargo grid takes.
    pub max_box: Option<u32>,
    /// Quantum drive cruise speed, megametres per second.
    pub qt_speed_mms: f64,
    /// Spool time per quantum jump, seconds.
    pub spool_seconds: f64,
}

impl Default for Ship {
    fn default() -> Self {
        Self {
            name: "My ship".into(),
            capacity_estimate: None,
            hard_max: None,
            max_box: None,
            qt_speed_mms: 200.0,
            spool_seconds: 5.0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    /// Where the ship is now, as a place name.
    pub start: String,
    /// Landing, loading and take-off at every stop; also the time to cross a jump point.
    pub stop_minutes: f64,
    /// Travel between two places on or around the same planet or moon.
    pub hop_minutes: f64,
    pub goal: Goal,
    /// Contracts already accepted in game (they count towards the 10 a player can hold).
    pub accepted: u32,
    /// Routes to return.
    pub alternatives: usize,
    /// Selection search width: sets kept per size, additions tried per set.
    pub beam: usize,
    pub expand: usize,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            start: String::new(),
            stop_minutes: 5.0,
            hop_minutes: 3.0,
            goal: Goal::RatePerHour { min_minutes: 20.0 },
            accepted: 0,
            alternatives: 5,
            beam: 24,
            expand: 8,
        }
    }
}

impl Settings {
    pub fn new(start: &str) -> Self {
        Self {
            start: start.to_owned(),
            ..Self::default()
        }
    }
}
