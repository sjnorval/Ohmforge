//! The `~` log console: what the simulation, the supply, the meters, the
//! firmware and the app report, in one place. [`log`] is the buffer
//! (testable without egui); the slide-over panel is gui-only.

pub mod log;
#[cfg(feature = "gui")]
mod panel;

#[cfg(feature = "gui")]
pub use panel::{Console, DEFAULT_WIDTH, SimState};
