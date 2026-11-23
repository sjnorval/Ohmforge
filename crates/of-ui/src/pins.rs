//! Thin wrappers around [`of_soc_esp32c3::pad`] for the desktop demos.
//!
//! The demos still assume refdes `U1` and solver ground node 0. New circuits
//! should use [`of_soc_esp32c3::Esp32c3Component`] (E3) so the return path
//! goes through the chip's own `GND` pin.

use of_analog::{Circuit, CircuitError, Var};
use of_soc_esp32c3::gpio::PinOut;
use of_soc_esp32c3::pad;

/// Demo chip reference designator (matches the historical `u1_gpio*` names).
const REFDES: &str = "U1";

/// Two zero-volt pin drivers: a resistor to `vdd` and a resistor to ground.
pub fn add_pin_drivers(
    circuit: &mut Circuit,
    pin: u8,
    gpio: Var,
    vdd: Var,
    initial: PinOut,
) -> Result<(), CircuitError> {
    pad::add_pin_drivers(circuit, REFDES, pin, gpio, vdd, 0, initial)
}

/// Register every GPIO event code for `pin` as a high/low-side resistance change.
pub fn hook_pin(analog: &mut of_analog::AnalogEngine, pin: u8) {
    pad::hook_pin(analog, REFDES, pin);
}
