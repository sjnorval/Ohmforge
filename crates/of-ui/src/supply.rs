//! Configurable bench-style PSU that feeds the simulated `vdd` rail.

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
    /// Current delivered into the rail (A), instantaneous.
    pub amps: f64,
    /// Period-averaged current (A) for the instrument face and sidebar.
    /// `None` means "use [`Self::amps`]".
    pub amps_avg: Option<f64>,
    /// Regulation mode.
    pub mode: SupplyMode,
    /// Set point or rail is outside 3.0–3.6 V.
    pub outside_rated: bool,
}

impl SupplyStatus {
    /// Current shown on the PSU face: a period average when the demo has
    /// filled [`Self::amps_avg`], otherwise the instantaneous value.
    pub fn display_amps(self) -> f64 {
        self.amps_avg.unwrap_or(self.amps)
    }
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

    // The PSU model itself is the engine's `bench.psu` (`of-components`); its
    // CV/CC, off and return-current behaviour is tested there, in
    // `of-engine/tests` and on the desktop demos in `session_sim`.

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
}
