//! Windows-only file and volume adapters; raw handles never escape their owners.
//!
//! `volume` implements the snapshot control boundary. `deletion` pins and verifies
//! file identities, `marker` owns probe files, and `handles` opens native handles.

pub(crate) mod deletion;
mod handles;
pub(crate) mod marker;
pub(crate) mod volume;
