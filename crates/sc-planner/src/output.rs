//! What [`plan`](crate::plan()) returns, ready to show: ranked routes with their stops and
//! cargo, and what became of each contract.

use std::fmt;

use crate::travel::Leg;

#[derive(Debug, Clone, PartialEq)]
pub struct PlanOutput {
    /// The goal in words.
    pub goal: String,
    pub start: String,
    pub stop_minutes: f64,
    pub hop_minutes: f64,
    /// Contracts a route may hold (10 minus those already accepted).
    pub max_contracts: usize,
    /// Best first.
    pub routes: Vec<RouteOut>,
    /// One per contract, same order.
    pub contracts: Vec<ContractOut>,
    pub issues: Vec<String>,
    /// Game version of the universe data, when known.
    pub data_version: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// Peak cargo at most 80% of the estimated capacity.
    Comfortable,
    Tight,
    OverEstimate,
    /// No capacity estimate set.
    Unknown,
}

#[derive(Debug, Clone, PartialEq)]
pub struct RouteOut {
    pub rank: usize,
    /// What this route is best at among those shown ("Best pay per hour", ...).
    pub label: String,
    /// Contract numbers (1-based, board order).
    pub contracts: Vec<usize>,
    pub reward: u64,
    pub reward_per_hour: u64,
    pub total_minutes: f64,
    pub quantum_minutes: f64,
    pub stop_minutes: f64,
    pub stops: usize,
    pub peak_scu: f64,
    /// Cargo already on board when the route starts.
    pub on_board_at_start: Vec<CargoMove>,
    pub fit: Fit,
    /// True when the stop order is proven shortest.
    pub exact_order: bool,
    pub warnings: Vec<String>,
    pub steps: Vec<StepOut>,
}

/// Cargo of one contract line loaded or delivered at a stop.
#[derive(Debug, Clone, PartialEq)]
pub struct CargoMove {
    /// Contract number, 1-based.
    pub contract: usize,
    /// Cargo line (flow) within the contract, 1-based.
    pub line: usize,
    pub commodity: String,
    pub scu: f64,
    pub restricted: bool,
    /// Share of a line with several pickups: the real split is unknown.
    pub estimated_split: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct StepOut {
    pub n: usize,
    pub location: String,
    pub system: String,
    pub drop_only: bool,
    /// Travel from the previous stop (or the start).
    pub travel: Leg,
    /// Minutes from departure to arrival here.
    pub arrive_minutes: f64,
    pub delivered: Vec<CargoMove>,
    pub picked: Vec<CargoMove>,
    pub cargo_after: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ContractStatus {
    TopRoute,
    /// In alternative route #n (1-based rank).
    Alternative(usize),
    LeftOut(String),
    NotInAnyRoute,
}

impl fmt::Display for ContractStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TopRoute => f.write_str("In the top route"),
            Self::Alternative(n) => write!(f, "In alternative #{n}"),
            Self::LeftOut(why) => write!(f, "Left out: {why}"),
            Self::NotInAnyRoute => f.write_str("Not in any route"),
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ContractOut {
    /// Contract number, 1-based.
    pub contract: usize,
    pub title: Option<String>,
    pub reward: Option<u64>,
    pub scu: u32,
    pub status: ContractStatus,
}
