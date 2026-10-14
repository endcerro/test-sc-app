use std::fmt;
use std::ops::{Add, Sub};

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::LocationId;

/// A point in a star system's frame, in metres. The star is at the origin.
#[derive(Debug, Clone, Copy, PartialEq, Default, Serialize, Deserialize)]
pub struct Vec3 {
    pub x: f64,
    pub y: f64,
    pub z: f64,
}

impl Vec3 {
    pub const ZERO: Self = Self::new(0.0, 0.0, 0.0);

    pub const fn new(x: f64, y: f64, z: f64) -> Self {
        Self { x, y, z }
    }

    pub fn length(self) -> f64 {
        (self.x * self.x + self.y * self.y + self.z * self.z).sqrt()
    }

    pub fn distance_to(self, other: Self) -> f64 {
        (self - other).length()
    }
}

impl Add for Vec3 {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self::new(self.x + rhs.x, self.y + rhs.y, self.z + rhs.z)
    }
}

impl Sub for Vec3 {
    type Output = Self;

    fn sub(self, rhs: Self) -> Self {
        Self::new(self.x - rhs.x, self.y - rhs.y, self.z - rhs.z)
    }
}

/// A distance in metres. `Display` picks a readable unit (m, km, Mm, Gm).
#[derive(Debug, Clone, Copy, PartialEq, PartialOrd, Default, Serialize, Deserialize)]
pub struct Distance(pub f64);

impl Distance {
    pub const ZERO: Self = Self(0.0);

    pub fn metres(self) -> f64 {
        self.0
    }

    pub fn km(self) -> f64 {
        self.0 / 1_000.0
    }

    /// Gigametres, the unit the in-game quantum drive UI uses.
    pub fn gm(self) -> f64 {
        self.0 / 1e9
    }
}

impl Add for Distance {
    type Output = Self;

    fn add(self, rhs: Self) -> Self {
        Self(self.0 + rhs.0)
    }
}

impl fmt::Display for Distance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let m = self.0.abs();
        if m >= 1e9 {
            write!(f, "{:.2} Gm", self.0 / 1e9)
        } else if m >= 1e6 {
            write!(f, "{:.2} Mm", self.0 / 1e6)
        } else if m >= 1e3 {
            write!(f, "{:.1} km", self.0 / 1e3)
        } else {
            write!(f, "{:.0} m", self.0)
        }
    }
}

/// One step of a [`Route`].
#[derive(Debug, Clone, PartialEq)]
pub enum Leg {
    /// Travel inside one system (quantum / sublight), straight line.
    Travel {
        from: LocationId,
        to: LocationId,
        distance: Distance,
    },
    /// Traverse a jump tunnel between two systems. Distance is not meaningful here.
    Jump {
        from: LocationId,
        to: LocationId,
        fuel_cost: Option<f64>,
    },
}

/// Path between two locations, possibly crossing systems through jump points.
#[derive(Debug, Clone, PartialEq)]
pub struct Route {
    pub legs: Vec<Leg>,
    /// Sum of all [`Leg::Travel`] distances (jumps excluded).
    pub total: Distance,
}

impl Route {
    pub fn jumps(&self) -> usize {
        self.legs
            .iter()
            .filter(|l| matches!(l, Leg::Jump { .. }))
            .count()
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum DistanceError {
    #[error("location {0} has no known position")]
    NoPosition(LocationId),
    #[error("locations {0} and {1} are in different systems")]
    DifferentSystems(LocationId, LocationId),
    #[error("no jump route between the systems of {0} and {1}")]
    NoJumpRoute(LocationId, LocationId),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vec_distance() {
        let a = Vec3::new(1.0, 2.0, 2.0);
        assert!((a.length() - 3.0).abs() < 1e-12);
        assert!((a.distance_to(Vec3::ZERO) - 3.0).abs() < 1e-12);
    }

    #[test]
    fn distance_display_units() {
        assert_eq!(Distance(12.0).to_string(), "12 m");
        assert_eq!(Distance(150_400.0).to_string(), "150.4 km");
        assert_eq!(Distance(52_659_000.0).to_string(), "52.66 Mm");
        assert_eq!(Distance(12_850_457_093.0).to_string(), "12.85 Gm");
    }
}
