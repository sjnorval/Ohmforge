//! The shipped demo circuits (`circuits/demos`, feature `demos`), embedded
//! with the firmware they name so an app runs them without the repository.
//!
//! The files keep the ESP32-C3's supply current from its datasheet (v2.5,
//! Table 5-8, 160 MHz): `quiescent_a = 0.023` with the CPU running, and
//! `0.016` for `button-irq`, which sleeps in `wfi`. The mask ROM and bootloader are never embedded:
//! add them with [`EmbeddedResolver::with_rom`] for the ROM-booted demos.
//! The parts they place are embedded too ([`PARTS`], [`parts`]), so an app
//! can build them without the repository's `parts/` directory.

use crate::embedded::EmbeddedResolver;
use crate::library::PartsIndex;

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
pub const DEMOS: [DemoFile; 9] = [
    demo!("blink"),
    demo!("dimmer"),
    demo!("fade"),
    demo!("button"),
    demo!("button-irq"),
    demo!("ne555-astable"),
    demo!("ne555-monostable"),
    demo!("ne555-pwm-dimmer"),
    demo!("esp32c3-555-clock"),
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
    "esp32c3-clock-counter/clock.elf",
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

macro_rules! parts {
    ($($dir:literal),* $(,)?) => {
        &[$(($dir, include_str!(concat!("../../../parts/", $dir, "/part.toml")))),*]
    };
}

/// The `part.toml` of every part the demos place, keyed by its directory
/// under `parts/`. Their SPICE libraries are in [`MODELS`].
pub const PARTS: &[(&str, &str)] = parts![
    "bench/psu",
    "discretes/1n4148",
    "leds/red-0603",
    "mcu/esp32-c3",
    "passives/bourns-ptv09a",
    "passives/capacitor-mlcc",
    "passives/capacitor-tantalum",
    "passives/resistor",
    "regulators/ams1117",
    "switches/pushbutton-6mm",
    "timers/ne555",
];

/// The SPICE libraries the embedded parts name (`model = "spice:<file>"`),
/// keyed by part directory and file name.
pub const MODELS: &[(&str, &str, &str)] = &[
    (
        "regulators/ams1117",
        "ams1117.lib",
        include_str!("../../../parts/regulators/ams1117/ams1117.lib"),
    ),
    (
        "timers/ne555",
        "ne555.lib",
        include_str!("../../../parts/timers/ne555/ne555.lib"),
    ),
];

/// The embedded SPICE library text for a part's `model` (`spice:<file>`),
/// or `None` for a built-in (`rust:`) model.
pub fn spice_model(model: &str) -> Option<&'static str> {
    let file = model.strip_prefix("spice:")?;
    MODELS.iter().find(|(_, f, _)| *f == file).map(|(_, _, text)| *text)
}

/// The embedded demo parts, parsed.
pub fn part_files() -> Result<Vec<of_parts::PartFile>, String> {
    PARTS
        .iter()
        .map(|(dir, text)| of_parts::parse_part(text).map_err(|e| format!("parts/{dir}/part.toml: {e}")))
        .collect()
}

/// The embedded demo parts as an index for [`crate::build::build`].
pub fn parts() -> Result<PartsIndex, String> {
    part_files().map(PartsIndex::new)
}
