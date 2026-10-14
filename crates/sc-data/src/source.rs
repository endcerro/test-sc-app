use sc_universe::{BuildWarning, Universe};

use crate::DataError;

/// Something that can produce a [`Universe`].
pub trait UniverseSource {
    fn load(&self) -> Result<LoadedUniverse, DataError>;
}

/// Result of a load: the universe plus anything worth surfacing to the user or the logs.
#[derive(Debug)]
pub struct LoadedUniverse {
    pub universe: Universe,
    /// Data quality issues found while assembling the tree.
    pub warnings: Vec<BuildWarning>,
    /// Operational notes (e.g. "network unavailable, using cached data from ...").
    pub notes: Vec<String>,
}

/// Knobs for how raw data is turned into a universe.
/// Defaults drop both kinds of noise.
#[derive(Debug, Clone, Copy, Default)]
pub struct LoadOptions {
    /// Keep hidden asteroid scatter objects (hundreds of unnamed, non-QT rocks).
    pub include_asteroid_scatter: bool,
    /// Keep objects that are hidden on the starmap, have no coordinates and no amenities
    /// (hundreds of mission spawn templates such as "QV Logistics Station").
    pub include_hidden_unpositioned: bool,
    /// Don't derive venue aliases from `labels.json` (and don't download it). Curated
    /// aliases are always applied.
    pub skip_label_aliases: bool,
}
