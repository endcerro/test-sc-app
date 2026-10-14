use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Dense index of a [`crate::Location`] inside a [`crate::Universe`].
///
/// Only valid for the universe that produced it; use [`GameUuid`] for anything that
/// must survive a reload.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct LocationId(u32);

impl LocationId {
    pub(crate) fn from_index(index: usize) -> Self {
        // A universe has a few thousand locations; overflowing u32 is a logic error.
        Self(u32::try_from(index).unwrap_or(u32::MAX))
    }

    /// Position of the location in [`crate::Universe::locations`].
    pub fn index(self) -> usize {
        self.0 as usize
    }
}

impl fmt::Display for LocationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "#{}", self.0)
    }
}

/// Identifier of an object in the game files (starmap object UUID).
///
/// Stable across reloads and game patches, so it is the key to join other sources on.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GameUuid(pub Uuid);

impl GameUuid {
    /// Parses a hyphenated UUID string.
    pub fn parse(s: &str) -> Result<Self, uuid::Error> {
        Uuid::parse_str(s).map(Self)
    }
}

impl fmt::Display for GameUuid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}
