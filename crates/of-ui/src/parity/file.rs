//! The demos built from their circuit files (`circuits/demos`, embedded by
//! `of_circuit::demos`) and run on `Session` with the parity sampling loop.
//! This is the E4b-2 half of trace parity; since E4c the build is the
//! desktop's own (`session_sim::build_session`: embedded parts and firmware,
//! the pre-boot caches).

use super::{Demo, DemoRun, session};
use crate::session_sim::{StartOptions, build_session};

/// The ESP32-C3 idle draw of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdleDraw {
    /// `quiescent_a = 0` (parity runs and the goldens).
    Off,
    /// The shipped file's `quiescent_a` (23 mA on blink; the idle-draw golden).
    AsShipped,
}

/// Build `demo` from its circuit file.
pub(crate) fn start(demo: Demo, idle: IdleDraw) -> Result<Box<dyn DemoRun>, String> {
    let opts = StartOptions {
        idle_draw: idle == IdleDraw::AsShipped,
        ..StartOptions::default()
    };
    session::run(build_session(demo.kind(), opts)?, demo)
}
