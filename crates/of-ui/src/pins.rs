//! Thin wrappers around [`of_soc_esp32c3::pad`] for the desktop demos.
//!
//! The demos still assume refdes `U1` and solver ground node 0. New circuits
//! should use [`of_soc_esp32c3::Esp32c3Component`] (E3) so the return path
//! goes through the chip's own `GND` pin.

use of_analog::devices::Resistor;
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

/// ESP32-C3 supply current for firmware that keeps the CPU running at
/// 160 MHz (blink, dimmer, fade, button poll): 23 mA, datasheet v2.5 Table 5-8.
pub use of_soc_esp32c3::QUIESCENT_A as IDLE_DRAW_RUNNING_A;
/// ESP32-C3 supply current for firmware that sleeps in `wfi` at 160 MHz
/// (button-irq): 16 mA, datasheet v2.5 Table 5-8.
pub use of_soc_esp32c3::QUIESCENT_WFI_A as IDLE_DRAW_WFI_A;

/// The ESP32-C3's own supply current as a resistor from `vdd` to ground that
/// draws `amps` at 3.3 V, named and sized like the engine component's
/// `u1_iq` (`quiescent_a`); `amps <= 0` adds nothing (parity runs).
pub fn add_idle_draw(circuit: &mut Circuit, vdd: Var, amps: f64) -> Result<(), CircuitError> {
    if amps > 0.0 {
        circuit.add(Resistor::new("u1_iq", vdd, 0, 3.3 / amps))?;
    }
    Ok(())
}
