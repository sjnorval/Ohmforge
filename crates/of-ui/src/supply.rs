//! Configurable bench-style PSU that feeds the simulated `vdd` rail.

use of_analog::devices::{CurrentSource, PinDriver, SourceValue};
use of_analog::{AnalogEngine, Circuit, CircuitError, Var};

/// ESP32-C3 recommended supply range (datasheet VDD).
pub const RATED_MIN_V: f64 = 3.0;
/// ESP32-C3 recommended supply range (datasheet VDD).
pub const RATED_MAX_V: f64 = 3.6;

/// Default set voltage (V).
pub const DEFAULT_VOLTS: f64 = 3.3;
/// Default series resistance (Ω).
pub const DEFAULT_SERIES_R: f64 = 0.1;
/// Default current limit (A). Generous so the LED and pot do not clamp.
pub const DEFAULT_LIMIT_A: f64 = 0.5;

/// User-settable bench supply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SupplySettings {
    /// Output enabled.
    pub on: bool,
    /// Constant-voltage set point (V).
    pub volts: f64,
    /// Internal / series resistance (Ω).
    pub series_r: f64,
    /// Current limit (A). The supply droops when this would be exceeded.
    pub current_limit: f64,
}

impl Default for SupplySettings {
    fn default() -> Self {
        Self {
            on: true,
            volts: DEFAULT_VOLTS,
            series_r: DEFAULT_SERIES_R,
            current_limit: DEFAULT_LIMIT_A,
        }
    }
}

impl SupplySettings {
    /// Clamp settings to a range the solver can handle.
    pub fn sanitized(self) -> Self {
        Self {
            on: self.on,
            volts: self.volts.clamp(0.0, 5.0),
            series_r: self.series_r.clamp(1e-4, 1e3),
            current_limit: self.current_limit.clamp(1e-6, 2.0),
        }
    }

    /// True when the set voltage is outside the ESP32-C3 recommended range.
    #[allow(dead_code)]
    pub fn set_outside_rated(self) -> bool {
        self.on && !(RATED_MIN_V..=RATED_MAX_V).contains(&self.volts)
    }
}

/// How the supply is regulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SupplyMode {
    /// Output is off (high-Z).
    Off,
    /// Holding the set voltage (plus the drop on the series resistance).
    #[default]
    Cv,
    /// Clamped at the current limit; the rail voltage has drooped.
    Cc,
}

/// Live output of the PSU.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SupplyStatus {
    /// Voltage on the `vdd` rail (V).
    pub volts: f64,
    /// Current delivered into the rail (A).
    pub amps: f64,
    /// Regulation mode.
    pub mode: SupplyMode,
    /// Set point or rail is outside 3.0–3.6 V.
    pub outside_rated: bool,
}

/// Add the PSU devices: a Thevenin source `psu` and a current source `psu_cc`.
pub fn add_supply(circuit: &mut Circuit, vdd: Var, settings: &SupplySettings) -> Result<(), CircuitError> {
    let s = settings.sanitized();
    let (v, r) = if s.on {
        (s.volts, s.series_r)
    } else {
        (0.0, PinDriver::HI_Z)
    };
    circuit.add(PinDriver::new("psu", vdd, 0, v, r))?;
    circuit.add(CurrentSource::new("psu_cc", 0, vdd, SourceValue::dc(0.0)))?;
    Ok(())
}

/// Apply `settings` in `mode` (no solve). Call [`regulate`] after the next step.
pub fn apply(analog: &mut AnalogEngine, settings: &SupplySettings, mode: SupplyMode) -> Result<SupplyMode, String> {
    let s = settings.sanitized();
    if !s.on {
        analog.set("psu", "v", 0.0).map_err(|e| e.to_string())?;
        analog.set("psu", "r", PinDriver::HI_Z).map_err(|e| e.to_string())?;
        analog.set("psu_cc", "dc", 0.0).map_err(|e| e.to_string())?;
        return Ok(SupplyMode::Off);
    }
    match mode {
        SupplyMode::Cc => {
            analog.set("psu", "v", 0.0).map_err(|e| e.to_string())?;
            analog.set("psu", "r", PinDriver::HI_Z).map_err(|e| e.to_string())?;
            analog.set("psu_cc", "dc", s.current_limit).map_err(|e| e.to_string())?;
            Ok(SupplyMode::Cc)
        }
        SupplyMode::Cv | SupplyMode::Off => {
            analog.set("psu", "v", s.volts).map_err(|e| e.to_string())?;
            analog.set("psu", "r", s.series_r).map_err(|e| e.to_string())?;
            analog.set("psu_cc", "dc", 0.0).map_err(|e| e.to_string())?;
            Ok(SupplyMode::Cv)
        }
    }
}

/// Read the rail, switch CV/CC if needed, and return the status.
///
/// Mode changes take effect on the next analog step (one sample of lag).
pub fn regulate(
    analog: &mut AnalogEngine,
    settings: &SupplySettings,
    mode: SupplyMode,
) -> Result<SupplyStatus, String> {
    let s = settings.sanitized();
    let v = analog.value("v(vdd)").unwrap_or(0.0);
    if !s.on {
        let _ = apply(analog, &s, SupplyMode::Off)?;
        return Ok(SupplyStatus {
            volts: v,
            amps: 0.0,
            mode: SupplyMode::Off,
            outside_rated: !(RATED_MIN_V..=RATED_MAX_V).contains(&v) && v > 0.05,
        });
    }

    let i_cv = (s.volts - v) / s.series_r;
    let next = match mode {
        SupplyMode::Off | SupplyMode::Cv => {
            if i_cv > s.current_limit {
                SupplyMode::Cc
            } else {
                SupplyMode::Cv
            }
        }
        SupplyMode::Cc => {
            // Compliance: if the rail would rise above the set point, go back to CV.
            if v > s.volts { SupplyMode::Cv } else { SupplyMode::Cc }
        }
    };
    let applied = if next == mode { next } else { apply(analog, &s, next)? };
    let amps = match applied {
        SupplyMode::Cc => s.current_limit,
        SupplyMode::Cv => i_cv.max(0.0),
        SupplyMode::Off => 0.0,
    };
    Ok(SupplyStatus {
        volts: v,
        amps,
        mode: applied,
        outside_rated: !(RATED_MIN_V..=RATED_MAX_V).contains(&s.volts) || !(RATED_MIN_V..=RATED_MAX_V).contains(&v),
    })
}

/// Short warning when the supply is outside the modelled chip's ratings.
pub fn rating_warning(status: &SupplyStatus) -> Option<&'static str> {
    status
        .outside_rated
        .then_some("outside ESP32-C3 rated supply (3.0–3.6 V); brownout/undervoltage is not modelled")
}

/// Mode label for the live readout.
pub fn mode_label(mode: SupplyMode) -> &'static str {
    match mode {
        SupplyMode::Off => "OFF",
        SupplyMode::Cv => "CV",
        SupplyMode::Cc => "CC",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use of_analog::devices::Resistor;
    use of_analog::{EngineConfig, Options};
    use of_core::{EngineId, Kernel, SimTime, Stop};

    fn load_board(r_load: f64, settings: SupplySettings) -> AnalogEngine {
        let mut c = Circuit::new();
        let vdd = c.node("vdd");
        c.add(Resistor::new("load", vdd, 0, r_load)).unwrap();
        add_supply(&mut c, vdd, &settings).unwrap();
        AnalogEngine::new(
            "analog",
            c,
            Options::default(),
            EngineConfig {
                max_step: 1e-5,
                ..EngineConfig::default()
            },
        )
        .unwrap()
    }

    fn run(analog: AnalogEngine, t: f64) -> Kernel {
        let mut k = Kernel::new(SimTime::from_us(10));
        k.add_engine(Box::new(analog));
        let end = SimTime::from_secs_f64(t);
        assert_eq!(k.run_until(end).unwrap(), Stop::Reached(end));
        k
    }

    #[test]
    fn resistive_load_scales_with_voltage() {
        let mut settings = SupplySettings {
            volts: 3.3,
            ..SupplySettings::default()
        };
        let mut k = run(load_board(330.0, settings), 100e-6);
        let v33 = k
            .engine_as::<AnalogEngine>(EngineId(0))
            .unwrap()
            .value("v(vdd)")
            .unwrap();
        assert!((v33 - 3.3).abs() < 0.02, "{v33}");
        drop(k);

        settings.volts = 5.0;
        let mut k = run(load_board(330.0, settings), 100e-6);
        let v50 = k
            .engine_as::<AnalogEngine>(EngineId(0))
            .unwrap()
            .value("v(vdd)")
            .unwrap();
        assert!((v50 - 5.0).abs() < 0.03, "{v50}");
        assert!((v50 / v33 - 5.0 / 3.3).abs() < 0.02);
    }

    #[test]
    fn current_limit_clamps_and_droops_the_rail() {
        let settings = SupplySettings {
            on: true,
            volts: 3.3,
            series_r: 0.1,
            current_limit: 1e-3,
        };
        // 330 Ω at 3.3 V would draw 10 mA; the 1 mA limit should droop the rail
        // to about 0.33 V.
        let analog = load_board(330.0, settings);
        let mut k = run(analog, 50e-6);
        let mode = {
            let a = k.engine_as::<AnalogEngine>(EngineId(0)).unwrap();
            regulate(a, &settings, SupplyMode::Cv).unwrap().mode
        };
        let t = SimTime::from_secs_f64(100e-6);
        assert_eq!(k.run_until(t).unwrap(), Stop::Reached(t));
        let status = {
            let a = k.engine_as::<AnalogEngine>(EngineId(0)).unwrap();
            regulate(a, &settings, mode).unwrap()
        };
        assert_eq!(status.mode, SupplyMode::Cc, "{status:?}");
        assert!((status.amps - 1e-3).abs() < 5e-5, "{status:?}");
        assert!((status.volts - 0.33).abs() < 0.05, "expected droop, got {status:?}");
    }

    #[test]
    fn off_is_high_impedance() {
        let settings = SupplySettings {
            on: false,
            ..SupplySettings::default()
        };
        let mut k = run(load_board(330.0, settings), 100e-6);
        let v = k
            .engine_as::<AnalogEngine>(EngineId(0))
            .unwrap()
            .value("v(vdd)")
            .unwrap();
        assert!(v.abs() < 0.05, "rail should collapse when the PSU is off, got {v}");
    }

    #[test]
    fn warning_when_outside_rated_range() {
        let mut s = SupplySettings::default();
        assert!(!s.set_outside_rated());
        s.volts = 2.5;
        assert!(s.set_outside_rated());
        s.volts = 5.0;
        assert!(s.set_outside_rated());
        s.on = false;
        assert!(!s.set_outside_rated());
    }

    #[test]
    fn sourced_current_returns_through_ground_and_includes_the_load() {
        use crate::pins::{add_pin_drivers, pin_driver_names};
        use of_soc_esp32c3::gpio::PinOut;

        let settings = SupplySettings::default();
        let mut c = Circuit::new();
        let vdd = c.node("vdd");
        let gpio = c.node("gpio8");
        c.add(Resistor::new("rled", gpio, 0, 330.0)).unwrap();
        c.add(Resistor::new("pot", vdd, 0, 10_000.0)).unwrap();
        add_supply(&mut c, vdd, &settings).unwrap();
        add_pin_drivers(&mut c, 8, gpio, vdd, PinOut::driven(true, 2)).unwrap();
        let analog = AnalogEngine::new(
            "analog",
            c,
            Options::default(),
            EngineConfig {
                max_step: 1e-5,
                ..EngineConfig::default()
            },
        )
        .unwrap();
        let mut k = run(analog, 100e-6);
        {
            let a = k.engine_as::<AnalogEngine>(EngineId(0)).unwrap();
            let i_psu = a.value("i(psu)").expect("i(psu)");
            let v_vdd = a.value("v(vdd)").unwrap();
            let v_gpio = a.value("v(gpio8)").unwrap();
            let i_led = v_gpio / 330.0;
            let i_pot = v_vdd / 10_000.0;
            assert!(
                (i_psu - (i_led + i_pot)).abs() < 1e-6,
                "KCL at VDD: i(psu)={i_psu} I_led={i_led} I_pot={i_pot}"
            );
            assert!(
                i_psu > 8e-3,
                "high-side LED current should flow from the PSU, got {i_psu} A"
            );
            let status = regulate(a, &settings, SupplyMode::Cv).unwrap();
            assert!(
                (status.amps - i_psu).abs() < 2e-5,
                "readout {} vs i(psu) {i_psu}",
                status.amps
            );

            let (hs, ls) = pin_driver_names(8);
            a.set(&hs, "r", PinDriver::HI_Z).unwrap();
            a.set(&ls, "r", 120.0).unwrap();
        }
        let t = SimTime::from_secs_f64(200e-6);
        assert_eq!(k.run_until(t).unwrap(), Stop::Reached(t));
        let a = k.engine_as::<AnalogEngine>(EngineId(0)).unwrap();
        let i_psu = a.value("i(psu)").expect("i(psu)");
        let v_vdd = a.value("v(vdd)").unwrap();
        let v_gpio = a.value("v(gpio8)").unwrap();
        let i_led = v_gpio / 330.0;
        let i_pot = v_vdd / 10_000.0;
        assert!(
            (i_psu - (i_led + i_pot)).abs() < 1e-6,
            "KCL with pin low: i(psu)={i_psu} I_led={i_led} I_pot={i_pot}"
        );
        assert!(
            (i_psu - i_pot).abs() < 1e-5,
            "low-side should leave only the divider on the PSU, i_psu={i_psu} i_pot={i_pot}"
        );
    }
}
