//! Data sources that build a [`sc_universe::Universe`].
//!
//! Today: [`ScunpackedSource`], backed by
//! [StarCitizenWiki/scunpacked-data](https://github.com/StarCitizenWiki/scunpacked-data)
//! (game files extracted per patch). It can read a local checkout or fetch the data once
//! at launch, with an on-disk cache used as fallback when offline.
//!
//! Later: UEX Corp (terminals, commodity prices) will plug in through [`UniverseSource`]
//! or as an enrichment pass on the loaded universe.

mod cache;
mod error;
pub mod http;
pub mod scunpacked;
mod source;

pub use cache::{Cache, CacheEntry};
pub use error::DataError;
pub use scunpacked::{RemoteConfig, ScunpackedSource};
pub use source::{LoadOptions, LoadedUniverse, UniverseSource};
