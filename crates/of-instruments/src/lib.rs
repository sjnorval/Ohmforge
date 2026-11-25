//! Oscilloscope, logic analyser, protocol decoders, multimeter, PSU.
//!
//! Engine E5 (#49, RFC 0004): the UI-free measurement service. A [`Signal`]
//! names anything measurable, a [`Probeable`] source (the engine `Session`
//! from E5b) samples it, and the [`MeasurementService`] records watched
//! signals into bounded [`ring::TraceBuf`] rings on a [`SamplePlan`]; [`session`]
//! makes the engine `Session` a [`Probeable`] (E5b).
//! [`stats`] holds the windowed and period-aligned averages, duty/frequency,
//! min/max and RMS. The serial monitor model lives in [`serial`]; egui panels
//! live in `of-ui`.

pub mod meter;
pub mod psu;
pub mod ring;
pub mod serial;
pub mod service;
pub mod session;
pub mod signal;
pub mod stats;

pub use meter::{Averaging, Candidate, MeterError, MeterMode, Multimeter, PartPins, Reading, candidates};
pub use psu::PsuReadout;
pub use ring::{SampleClock, TraceBuf};
pub use serial::{Delivery, Dir, Entry, FindHit, SendError, SerialMonitor};
pub use service::{Channel, ChannelId, MeasurementService, ProbeError, Probeable, SamplePlan};
pub use signal::{ParseSignalError, Signal};
