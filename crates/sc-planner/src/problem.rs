//! The planning problem in plain numbers: locations, travel minutes between them,
//! and jobs (contracts) made of cargo orders. No game data here, so the solver can be
//! tested against brute force on made-up worlds.

#[derive(Debug, Clone)]
pub struct Order {
    /// Cargo in SCU. Split evenly between the pickups (the real split is unknown).
    pub scu: f64,
    /// Location indexes. Empty when the cargo is already on board.
    pub pickups: Vec<usize>,
    pub drop: usize,
}

#[derive(Debug, Clone)]
pub struct Job {
    pub reward: f64,
    pub orders: Vec<Order>,
}

#[derive(Debug, Clone)]
pub struct Problem {
    /// travel[a][b]: minutes from a to b, not counting the stop at b.
    pub travel: Vec<Vec<f64>>,
    /// Where the ship is now.
    pub start: usize,
    /// Minutes spent at every stop.
    pub stop_minutes: f64,
    pub jobs: Vec<Job>,
    /// Cargo on board may never exceed this.
    pub hard_max: Option<f64>,
}

impl Problem {
    pub fn t(&self, a: usize, b: usize) -> f64 {
        self.travel[a][b]
    }

    pub fn fits(&self, load: f64) -> bool {
        self.hard_max.is_none_or(|m| load <= m + 1e-9)
    }
}
