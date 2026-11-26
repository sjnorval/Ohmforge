//! The demo boards built from the shipped circuit files, for the wiring
//! tests (the hand-written boards they replaced were deleted in E4d; the
//! route goldens pin the drawing).

use of_circuit::parse;

use super::from_circuit::{file_params, from_circuit};
use super::model::Schematic;

pub(crate) const BLINK: &str = include_str!("../../../../circuits/demos/blink.circuit.toml");
pub(crate) const DIMMER: &str = include_str!("../../../../circuits/demos/dimmer.circuit.toml");
pub(crate) const FADE: &str = include_str!("../../../../circuits/demos/fade.circuit.toml");
pub(crate) const BUTTON: &str = include_str!("../../../../circuits/demos/button.circuit.toml");
pub(crate) const BUTTON_IRQ: &str = include_str!("../../../../circuits/demos/button-irq.circuit.toml");
pub(crate) const NE555_ASTABLE: &str = include_str!("../../../../circuits/demos/ne555-astable.circuit.toml");
pub(crate) const NE555_MONOSTABLE: &str = include_str!("../../../../circuits/demos/ne555-monostable.circuit.toml");
pub(crate) const NE555_PWM: &str = include_str!("../../../../circuits/demos/ne555-pwm-dimmer.circuit.toml");
pub(crate) const ESP32C3_555_CLOCK: &str = include_str!("../../../../circuits/demos/esp32c3-555-clock.circuit.toml");

fn routed(text: &str) -> Schematic {
    from_circuit(&parse(text).expect("shipped file"), &file_params).expect("schematic")
}

pub fn blink_board() -> Schematic {
    routed(BLINK)
}

pub fn dimmer_board() -> Schematic {
    routed(DIMMER)
}

/// The dimmer with the pot wiper at `position` (clamped to 0..=1).
pub fn dimmer_board_at(position: f64) -> Schematic {
    let live = move |r: &str, p: &str| (r == "RV1" && p == "position").then_some(position);
    from_circuit(&parse(DIMMER).expect("shipped file"), &live).expect("schematic")
}

pub fn fade_board() -> Schematic {
    routed(FADE)
}

pub fn button_board() -> Schematic {
    routed(BUTTON)
}
