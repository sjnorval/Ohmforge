//! The shipped demo circuits (`circuits/demos`, feature `demos`), embedded
//! with the firmware they name so an app runs them without the repository.
//!
//! The files keep the ESP32-C3's true-to-life 5 mA idle draw
//! (`quiescent_a = 0.005`). The mask ROM and bootloader are never embedded:
//! add them with [`EmbeddedResolver::with_rom`] for the ROM-booted demos.

use crate::embedded::EmbeddedResolver;

/// One embedded demo file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DemoFile {
    /// Stem, for example `blink`.
    pub name: &'static str,
    /// Repository path, for example `circuits/demos/blink.circuit.toml`.
    pub path: &'static str,
    /// File text.
    pub text: &'static str,
}

macro_rules! demo {
    ($name:literal) => {
        DemoFile {
            name: $name,
            path: concat!("circuits/demos/", $name, ".circuit.toml"),
            text: include_str!(concat!("../../../circuits/demos/", $name, ".circuit.toml")),
        }
    };
}

/// Every shipped demo, in menu order.
pub const DEMOS: [DemoFile; 5] = [
    demo!("blink"),
    demo!("dimmer"),
    demo!("fade"),
    demo!("button"),
    demo!("button-irq"),
];

/// The demo with this stem.
pub fn demo(name: &str) -> Option<&'static DemoFile> {
    DEMOS.iter().find(|d| d.name == name)
}

macro_rules! firmware {
    ($($path:literal),* $(,)?) => {
        &[$((concat!("../../firmware/", $path), include_bytes!(concat!("../../../firmware/", $path)) as &[u8])),*]
    };
}

/// Firmware the demo files name, keyed by the path as written in them.
pub const FIRMWARE: &[(&str, &[u8])] = firmware![
    "esp32c3-blinky/blinky.elf",
    "esp32c3-esp-hal-adc-pwm/pot_dimmer.bin",
    "esp32c3-esp-hal-adc-pwm/led_fade.bin",
    "esp32c3-esp-hal-adc-pwm/partitions.bin",
    "esp32c3-esp-hal-button/button_poll.bin",
    "esp32c3-esp-hal-button/button_irq.bin",
    "esp32c3-esp-hal-button/partitions.bin",
];

/// A resolver holding every demo firmware file (no ROM).
pub fn resolver() -> EmbeddedResolver {
    FIRMWARE
        .iter()
        .fold(EmbeddedResolver::new(), |r, (path, bytes)| r.with_file(path, *bytes))
}
