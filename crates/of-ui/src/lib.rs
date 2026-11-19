//! egui views and instrument panels, enabled by the `gui` feature.

#[cfg(feature = "gui")]
mod blink;

#[cfg(feature = "gui")]
pub use blink::BlinkDemo;
