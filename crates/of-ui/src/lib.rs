//! egui views and instrument panels, enabled by the `gui` feature.

#[cfg(feature = "gui")]
mod blink;
#[cfg(feature = "gui")]
mod serial;

#[cfg(feature = "gui")]
pub use blink::BlinkDemo;
#[cfg(feature = "gui")]
pub use serial::SerialMonitorPanel;
