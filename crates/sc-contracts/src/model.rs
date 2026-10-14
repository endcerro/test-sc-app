use std::fmt;

/// One hauling contract offer.
///
/// Every field is optional because a screenshot may not show it (the details text can be
/// scrolled, the title box truncates long titles).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HaulingContract {
    /// Title as displayed, both lines joined with a space.
    pub title: Option<String>,
    /// Reward in aUEC.
    pub reward: Option<u64>,
    /// "Contracted By".
    pub contractor: Option<String>,
    /// "Contract Availability": `N/A` or a countdown such as `2h 1m`.
    pub availability: Option<String>,
    /// Largest cargo container the contract uses, in SCU (from the details text).
    pub max_box: Option<u32>,
    /// "Reputation Awarded" (from the details text).
    pub rep: Option<u32>,
    /// One entry per `Deliver` objective, in screen order.
    pub cargo: Vec<Cargo>,
}

/// One `Deliver N SCU of X to Y` objective with its `Collect` pickups.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cargo {
    pub scu: u32,
    /// Commodity name without the `[!]` marker.
    pub commodity: String,
    /// The `[!]` marker the game shows before restricted commodities.
    pub restricted: bool,
    /// Where to collect it. Several entries when the cargo is spread over several places.
    pub pickups: Vec<Place>,
    pub dropoff: Place,
}

/// A place name as written in the contract (`Ruin Station above Pyro VI`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Place {
    pub name: String,
    /// Not read from the screen but inferred (e.g. the Collect line was scrolled off).
    /// Written with a trailing `?` in truth files.
    pub uncertain: bool,
}

impl Place {
    pub fn new(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            uncertain: false,
        }
    }

    pub fn uncertain(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            uncertain: true,
        }
    }
}

impl fmt::Display for Place {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.name)?;
        if self.uncertain {
            f.write_str("?")?;
        }
        Ok(())
    }
}

impl Cargo {
    /// Commodity as displayed, with the `[!]` marker.
    pub fn display_commodity(&self) -> String {
        if self.restricted {
            format!("[!] {}", self.commodity)
        } else {
            self.commodity.clone()
        }
    }
}
