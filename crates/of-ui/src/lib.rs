//! egui views and instrument panels, enabled by the `gui` feature.

#[cfg_attr(not(test), allow(dead_code, unused_imports))]
mod wiring;

#[cfg(feature = "gui")]
mod adc_pwm;
#[cfg(feature = "gui")]
mod bench;
#[cfg(feature = "gui")]
mod blink;
#[cfg(feature = "gui")]
mod boards;
#[cfg(feature = "gui")]
mod button;
#[cfg(feature = "gui")]
mod demo;
#[cfg(feature = "gui")]
mod digital_ui;
#[cfg(feature = "gui")]
mod kind;
#[cfg(feature = "gui")]
mod live;
#[cfg(feature = "gui")]
mod meters;
#[cfg(feature = "gui")]
mod meters_ui;
#[cfg(all(feature = "gui", test))]
mod parity;
#[cfg(feature = "gui")]
#[cfg(feature = "gui")]
pub mod prefs;
#[cfg(feature = "gui")]
mod rom;
#[cfg(feature = "gui")]
mod schematic;
#[cfg(feature = "gui")]
mod serial;
#[cfg(feature = "gui")]
mod session_sim;
#[cfg(feature = "gui")]
pub mod splash;
#[cfg(all(feature = "gui", test))]
mod stats;
#[cfg(feature = "gui")]
mod supply;
#[cfg(feature = "gui")]
mod theme;
#[cfg(feature = "gui")]
mod trace;

#[cfg(feature = "gui")]
pub use demo::OhmforgeDemo;
#[cfg(feature = "gui")]
pub use serial::SerialMonitorPanel;
