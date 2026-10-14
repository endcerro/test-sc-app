//! Star Citizen universe model.
//!
//! This crate is **pure**: no IO, no network, no filesystem. Data sources live in
//! `sc-data` and feed this crate through [`UniverseBuilder`].
//!
//! The model is a tree stored in an arena:
//!
//! ```text
//! System (synthetic) -> Star -> Planet -> Moon -> Station / City / Outpost / POI ...
//! ```
//!
//! Positions are in **metres**, expressed in the frame of the owning star system
//! (star at the origin). Distances between systems go through [`JumpLink`]s.

mod builder;
mod geometry;
mod id;
mod kind;
mod location;
mod lookup;
mod universe;

pub use builder::{BuildWarning, JumpSpec, NodeSpec, UniverseBuilder};
pub use geometry::{Distance, DistanceError, Leg, Route, Vec3};
pub use id::{GameUuid, LocationId};
pub use kind::LocationKind;
pub use location::{Amenity, Location, Position};
pub use lookup::{LookupOptions, Match, normalize};
pub use universe::{DataMeta, JumpLink, Universe};
