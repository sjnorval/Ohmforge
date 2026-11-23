//! BOM placements for the 0.1 desktop demos.
//!
//! These mirror the schematic references (U1, R1/R2, D1, RV1, SW1). Bench
//! PSU and instruments are omitted so they never appear on the costed list.

use crate::BomPlacement;

/// Blink board: ESP32-C3, series resistor, red LED.
pub fn blink() -> Vec<BomPlacement> {
    vec![
        BomPlacement::new("U1", "espressif.esp32-c3"),
        BomPlacement::new("R1", "generic.resistor"),
        BomPlacement::new("D1", "generic.led-red-0603"),
    ]
}

/// Pot dimmer board: chip, pot, series resistor (R2 on the schematic), LED.
pub fn dimmer() -> Vec<BomPlacement> {
    vec![
        BomPlacement::new("U1", "espressif.esp32-c3"),
        BomPlacement::new("RV1", "bourns.ptv09a-b103"),
        BomPlacement::new("R2", "generic.resistor"),
        BomPlacement::new("D1", "generic.led-red-0603"),
    ]
}

/// LED fade board: same LED net as the dimmer, no pot.
pub fn fade() -> Vec<BomPlacement> {
    vec![
        BomPlacement::new("U1", "espressif.esp32-c3"),
        BomPlacement::new("R2", "generic.resistor"),
        BomPlacement::new("D1", "generic.led-red-0603"),
    ]
}

/// Push-button board: chip, switch, series resistor, LED.
pub fn button() -> Vec<BomPlacement> {
    vec![
        BomPlacement::new("U1", "espressif.esp32-c3"),
        BomPlacement::new("SW1", "generic.pushbutton"),
        BomPlacement::new("R2", "generic.resistor"),
        BomPlacement::new("D1", "generic.led-red-0603"),
    ]
}

/// Named demo boards for the CLI.
pub fn by_name(name: &str) -> Option<Vec<BomPlacement>> {
    match name.to_ascii_lowercase().as_str() {
        "blink" => Some(blink()),
        "dimmer" => Some(dimmer()),
        "fade" => Some(fade()),
        "button" | "push-button" | "pushbutton" => Some(button()),
        _ => None,
    }
}

/// Demo names accepted by [`by_name`].
pub fn names() -> &'static [&'static str] {
    &["blink", "dimmer", "fade", "button"]
}
