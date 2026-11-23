//! GPIO pad half-bridge for the analog solver (high-side to 3V3, low-side to GND).
//!
//! Moved out of `of-ui` for E3 so the ESP32-C3 component owns the return path
//! through its own supply pins (design §3c).

use of_analog::devices::PinDriver;
use of_analog::{AnalogEngine, Circuit, CircuitError, SetParam, Var};

use crate::engine::all_pin_codes;
use crate::gpio::{PinOut, R_PULL};

/// A pad doing nothing is this many ohms to ground, so the net stays defined.
const R_FLOAT: f64 = 1e9;

/// Output resistance of the pad driver at `FUN_DRV` (about 120, 60, 30, 15 Ω).
fn r_drive(strength: u8) -> f64 {
    120.0 / f64::from(1u32 << strength.min(3))
}

/// High-side (pad to VDD) and low-side (pad to ground) resistances for `out`.
pub fn pin_legs(out: PinOut) -> (f64, f64) {
    let mut g_hs = 0.0;
    let mut g_ls = 0.0;
    if let Some(level) = out.level {
        let g = 1.0 / r_drive(out.strength);
        if level {
            g_hs += g;
        } else {
            g_ls += g;
        }
    }
    if out.pull_up {
        g_hs += 1.0 / R_PULL;
    }
    if out.pull_down {
        g_ls += 1.0 / R_PULL;
    }
    let hs = if g_hs > 0.0 { 1.0 / g_hs } else { PinDriver::HI_Z };
    let ls = if g_ls > 0.0 {
        1.0 / g_ls
    } else if g_hs > 0.0 {
        PinDriver::HI_Z
    } else {
        R_FLOAT
    };
    (hs, ls)
}

/// Names of the high-side and low-side pin drivers for `pin` on `refdes`.
pub fn pin_driver_names(refdes: &str, pin: u8) -> (String, String) {
    let base = refdes.to_ascii_lowercase();
    (format!("{base}_gpio{pin}_hs"), format!("{base}_gpio{pin}_ls"))
}

/// Two zero-volt pin drivers: a resistor to `vdd` and a resistor to `gnd`.
pub fn add_pin_drivers(
    circuit: &mut Circuit,
    refdes: &str,
    pin: u8,
    gpio: Var,
    vdd: Var,
    gnd: Var,
    initial: PinOut,
) -> Result<(), CircuitError> {
    let (hs, ls) = pin_legs(initial);
    let (hn, ln) = pin_driver_names(refdes, pin);
    circuit.add(PinDriver::new(&hn, vdd, gpio, 0.0, hs))?;
    circuit.add(PinDriver::new(&ln, gpio, gnd, 0.0, ls))?;
    Ok(())
}

/// Analog actions that apply `out` to the high-side and low-side drivers.
pub fn pin_actions(refdes: &str, pin: u8, out: PinOut) -> Vec<SetParam> {
    let (hs, ls) = pin_legs(out);
    let (hn, ln) = pin_driver_names(refdes, pin);
    vec![
        SetParam {
            device: hn,
            param: "r".into(),
            value: hs,
        },
        SetParam {
            device: ln,
            param: "r".into(),
            value: ls,
        },
    ]
}

/// Register every GPIO event code for `pin` as a high/low-side resistance change.
pub fn hook_pin(analog: &mut AnalogEngine, refdes: &str, pin: u8) {
    for (code, out) in all_pin_codes(pin) {
        analog.on_code(code, pin_actions(refdes, pin, out));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpio::PinOut;

    #[test]
    fn legs_match_the_pad_model() {
        let hi = PinOut::driven(true, 2);
        assert_eq!(pin_legs(hi), (30.0, PinDriver::HI_Z));
        let lo = PinOut::driven(false, 0);
        assert_eq!(pin_legs(lo), (PinDriver::HI_Z, 120.0));
        let hiz = PinOut::HIZ;
        assert_eq!(pin_legs(hiz), (PinDriver::HI_Z, R_FLOAT));
        let up = PinOut {
            pull_up: true,
            ..PinOut::HIZ
        };
        assert_eq!(pin_legs(up), (R_PULL, PinDriver::HI_Z));
        let both = PinOut {
            pull_up: true,
            pull_down: true,
            ..PinOut::HIZ
        };
        let (hs, ls) = pin_legs(both);
        assert!((hs - R_PULL).abs() < 1e-9 && (ls - R_PULL).abs() < 1e-9);
    }
}
