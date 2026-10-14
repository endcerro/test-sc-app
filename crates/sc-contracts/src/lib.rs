//! Hauling contracts as read from the in-game mobiGlas **Contracts > Offers** screen.
//!
//! This crate is **pure** (no IO, no images): it owns the [`HaulingContract`] model, the
//! hand-written `#HAULING v1` truth format ([`truth`]), parsers that turn OCR'd region text
//! into a contract ([`parse`]), merging of scrolled screenshots ([`merge`]), optional
//! dictionary snapping ([`snap`]) and scoring against truth ([`score`]).
//!
//! The screen has four regions of interest, OCR'd separately:
//!
//! ```text
//! title      "Rookie | DIRECT Interstellar Bulk Haul | Ruin Station > Baijini Point"
//! reward     values column of the Reward / Contract Availability / Contracted By box
//! details    free text: full place names, container size, reputation
//! objectives "Deliver 0/168 SCU of Ammonia to Baijini Point above ArcCorp."
//!            "Collect Ammonia from Ruin Station."
//! ```

pub mod merge;
mod model;
pub mod parse;
pub mod repair;
pub mod score;
pub mod snap;
pub mod truth;

pub use model::{Cargo, HaulingContract, Place};
pub use parse::{ObjectiveItem, RegionText};
pub use score::{Score, score};
