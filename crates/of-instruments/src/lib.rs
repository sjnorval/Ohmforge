//! Oscilloscope, logic analyser, protocol decoders, multimeter, PSU.
//!
//! Serial monitor model lives in [`serial`]; egui panels live in `of-ui`.

pub mod serial;

pub use serial::{Delivery, Dir, Entry, FindHit, SendError, SerialMonitor};
