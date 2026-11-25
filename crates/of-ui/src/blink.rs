mod schematic;

pub(super) use schematic::show as show_schematic;
pub(crate) const R_SERIES: f64 = 330.0;
pub(crate) const GPIO: u8 = 8;
pub(crate) const RESISTOR_REF: &str = "R1";
pub(crate) const LED_REF: &str = "D1";
