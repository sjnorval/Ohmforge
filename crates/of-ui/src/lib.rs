//! egui views and instrument panels, enabled by the `gui` feature.

#[cfg(feature = "gui")]
mod adc_pwm;
#[cfg(feature = "gui")]
mod blink;
#[cfg(feature = "gui")]
mod demo;
#[cfg(feature = "gui")]
mod rom;
#[cfg(feature = "gui")]
mod serial;
#[cfg(feature = "gui")]
mod theme;

#[cfg(feature = "gui")]
pub use demo::OhmforgeDemo;
#[cfg(feature = "gui")]
pub use serial::SerialMonitorPanel;
