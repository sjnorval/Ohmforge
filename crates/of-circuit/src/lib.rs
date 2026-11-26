//! Circuit files (`*.circuit.toml`, [RFC 0003](https://github.com/sjnorval/Ohmforge/blob/main/rfcs/0003-circuit-file.md)).
//!
//! One TOML file describes a circuit for both the simulation and the schematic:
//! components with typed parameters, firmware by path and SHA-256, nets in
//! router order, per-instance schematic geometry, keepouts and probes.
//!
//! - [`parse`] reads a file and runs the structural checks ([`diagnostics`]);
//!   every error carries a 1-based line and column ([`CircuitError`]).
//! - [`save`] writes the canonical form; canonical files round-trip byte for byte.
//! - Feature `engine`: [`library`] checks parts, pins and firmware hashes;
//!   [`build`] turns a file into an `of_engine` session through the model
//!   registry; [`embedded`] resolves firmware from memory.
//! - Feature `fs`: [`fs`] reads files, the parts directory and the fetched
//!   mask ROM from disk.
//! - Feature `demos`: [`demos`] embeds the shipped `circuits/demos` files and
//!   the firmware they name.
//!
//! The default build has no host I/O and builds for `wasm32-unknown-unknown`.
//! The files themselves are data, licensed CC-BY-SA-4.0 (`circuits/LICENSE.md`).

mod error;
mod format;
mod parse;
mod save;
mod validate;

#[cfg(feature = "engine")]
pub mod build;
#[cfg(feature = "demos")]
pub mod demos;
#[cfg(feature = "engine")]
pub mod embedded;
#[cfg(feature = "fs")]
pub mod fs;
#[cfg(feature = "engine")]
pub mod library;
#[cfg(feature = "engine")]
pub mod loads;

pub use error::CircuitError;
pub use format::*;
pub use parse::{parse, parse_unchecked};
pub use save::{same_content, save};
pub use validate::diagnostics;
