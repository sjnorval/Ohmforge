//! Bench PSU as a [`Component`] (RFC 0001 / design §3d).

use of_analog::devices::BenchSupply;
use of_component_api::{
    Component, ComponentEvent, ComponentInfo, DeviceRef, ElabCtx, ElabError, EventCtx, Health, PinKind, PinSpec,
    SolutionView,
};
use of_core::SimTime;

/// Default set voltage (V).
pub const DEFAULT_VOLTS: f64 = 3.3;
/// Default series resistance (Ω).
pub const DEFAULT_SERIES_R: f64 = 0.1;
/// Default current limit (A).
pub const DEFAULT_LIMIT_A: f64 = 0.5;
/// Highest set point (V): a Rigol DP832's 30 V channels (CH1/CH2), the
/// bench supply profile in `parts/instruments/rigol.dp832`.
pub const MAX_VOLTS: f64 = 30.0;

/// User-settable bench supply.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SupplySettings {
    /// Output enabled.
    pub on: bool,
    /// Constant-voltage set point (V).
    pub volts: f64,
    /// Internal / series resistance (Ω).
    pub series_r: f64,
    /// Current limit (A).
    pub current_limit: f64,
    /// Supply range of the parts on the + rail, `(min, max)` V, from their
    /// `[electrical]` `vdd`/`vcc` (the circuit builder passes it as the
    /// `rated_min`/`rated_max` params); `None` means no rating warning.
    pub rated: Option<(f64, f64)>,
}

impl Default for SupplySettings {
    fn default() -> Self {
        Self {
            on: true,
            volts: DEFAULT_VOLTS,
            series_r: DEFAULT_SERIES_R,
            current_limit: DEFAULT_LIMIT_A,
            rated: None,
        }
    }
}

impl SupplySettings {
    /// Clamp settings to a range the solver can handle.
    pub fn sanitized(self) -> Self {
        Self {
            on: self.on,
            volts: self.volts.clamp(0.0, MAX_VOLTS),
            series_r: self.series_r.clamp(1e-4, 1e3),
            current_limit: self.current_limit.clamp(1e-6, 2.0),
            rated: self.rated,
        }
    }

    /// True when the set voltage is outside the load's rated range.
    pub fn set_outside_rated(self) -> bool {
        self.on && self.rated.is_some_and(|(lo, hi)| !(lo..=hi).contains(&self.volts))
    }
}

/// Live output of the PSU, as its face shows it (built from the component's
/// readings by `of_instruments::psu::PsuReadout`).
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct SupplyStatus {
    /// Voltage on the `+` rail (V).
    pub volts: f64,
    /// Current delivered into the rail (A), instantaneous.
    pub amps: f64,
    /// Period-averaged current (A) for the instrument face and sidebar.
    /// `None` means "use [`Self::amps`]".
    pub amps_avg: Option<f64>,
    /// Regulation mode.
    pub mode: SupplyMode,
    /// Set point or rail is outside the supply range of the parts on the
    /// + rail (`rated_min`/`rated_max`, set from the circuit's parts).
    pub outside_rated: bool,
}

impl SupplyStatus {
    /// Current shown on the PSU face: the period average when filled,
    /// otherwise the instantaneous value.
    pub fn display_amps(self) -> f64 {
        self.amps_avg.unwrap_or(self.amps)
    }
}

/// How the supply is regulating.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SupplyMode {
    /// Output is off (high-Z).
    Off,
    /// Holding the set voltage.
    #[default]
    Cv,
    /// Clamped at the current limit.
    Cc,
}

/// Warning for a rail whose parts share no supply range (`lo` is the
/// highest minimum, `hi` the lowest maximum).
pub fn no_common_range(lo: f64, hi: f64) -> String {
    format!("parts on this rail have no common supply range (one needs at least {lo} V, another at most {hi} V)")
}

/// Bench PSU component with explicit `+` and `−` terminals.
pub struct BenchPsu {
    info: ComponentInfo,
    settings: SupplySettings,
    mode: SupplyMode,
    /// Device name prefix (refdes lowercased).
    device: String,
    plus_net: Option<String>,
    minus_net: Option<String>,
    /// The output device ([`BenchSupply`]: CV behind `series_r`, CC at the
    /// limit, never above the set point).
    refs: Option<DeviceRef>,
    /// Current reported at the last commit (A): the limit in CC, else the
    /// CV current `(volts - v) / series_r` floored at 0.
    amps: f64,
    /// Kernel time of the last host set-point change. A rail read at or
    /// before it was solved for the old set point.
    set_at: Option<SimTime>,
}

impl BenchPsu {
    /// Place a PSU as `refdes` with the given settings.
    pub fn new(refdes: &str, settings: SupplySettings) -> Self {
        Self {
            info: ComponentInfo {
                part_id: "bench.psu".into(),
                refdes: refdes.to_owned(),
                pins: vec![
                    PinSpec {
                        name: "+".into(),
                        kind: PinKind::Power,
                    },
                    PinSpec {
                        name: "-".into(),
                        kind: PinKind::Ground,
                    },
                ],
            },
            settings: settings.sanitized(),
            mode: if settings.on { SupplyMode::Cv } else { SupplyMode::Off },
            device: refdes.to_ascii_lowercase(),
            plus_net: None,
            minus_net: None,
            refs: None,
            amps: 0.0,
            set_at: None,
        }
    }

    /// Settings in use.
    pub fn settings(&self) -> SupplySettings {
        self.settings
    }

    /// Regulation mode after the last commit.
    pub fn mode(&self) -> SupplyMode {
        self.mode
    }

    /// Load the settings into the output device (no solve). The device
    /// regulates by itself (CV or CC, whichever gives less current); `mode`
    /// is what the face shows, and off when the output is off.
    fn apply(&self, mode: SupplyMode, cx: &mut dyn EventCtx) -> SupplyMode {
        let Some(dev) = self.refs else {
            return mode;
        };
        let s = self.settings;
        let _ = cx.set(dev, "v", s.volts);
        let _ = cx.set(dev, "r", s.series_r);
        let _ = cx.set(dev, "limit", s.current_limit);
        let _ = cx.set(dev, "on", if s.on { 1.0 } else { 0.0 });
        if s.on { mode } else { SupplyMode::Off }
    }

    fn device_name(&self) -> String {
        self.device.clone()
    }
}

impl Component for BenchPsu {
    fn info(&self) -> &ComponentInfo {
        &self.info
    }

    fn elaborate(&mut self, cx: &mut dyn ElabCtx) -> Result<(), ElabError> {
        let plus = cx.pin_node("+")?;
        let minus = cx.pin_node("-")?;
        self.plus_net = Some(net_name(cx.circuit_mut(), plus));
        self.minus_net = Some(net_name(cx.circuit_mut(), minus));

        if let Some(v) = cx.param("volts") {
            self.settings.volts = v;
        }
        if let Some(r) = cx.param("series_r") {
            self.settings.series_r = r;
        }
        if let Some(i) = cx.param("current_limit") {
            self.settings.current_limit = i;
        }
        if let Some(on) = cx.param("on") {
            self.settings.on = on != 0.0;
        }
        if let (Some(lo), Some(hi)) = (cx.param("rated_min"), cx.param("rated_max")) {
            self.settings.rated = Some((lo, hi));
        }
        // A cold start (`cold_start`, from `[sim] uic`) begins with every net
        // at 0 V, not a solved rail: like a fresh set point, regulate only
        // after the first step, or a commit at t = 0 would read a short and
        // force the current limit into the load.
        if cx.param("cold_start").is_some_and(|v| v != 0.0) {
            self.set_at = Some(SimTime::ZERO);
        }
        self.settings = self.settings.sanitized();

        let s = self.settings;
        let name = self.device_name();
        let dev = cx.add_device(Box::new(BenchSupply::new(
            &name,
            plus,
            minus,
            s.volts,
            s.series_r,
            s.current_limit,
            s.on,
        )))?;
        self.refs = Some(dev);
        self.mode = if s.on { SupplyMode::Cv } else { SupplyMode::Off };
        Ok(())
    }

    fn on_event(&mut self, ev: &ComponentEvent, cx: &mut dyn EventCtx) {
        let ComponentEvent::HostParam { name, value } = ev else {
            return;
        };
        match name.as_str() {
            "volts" => self.settings.volts = *value,
            "series_r" => self.settings.series_r = *value,
            "current_limit" => self.settings.current_limit = *value,
            "on" => self.settings.on = *value != 0.0,
            _ => return,
        }
        self.settings = self.settings.sanitized();
        self.set_at = Some(cx.now());
        // New set points take effect on the next step, in the current mode.
        let mode = if self.settings.on && self.mode == SupplyMode::Off {
            SupplyMode::Cv
        } else {
            self.mode
        };
        self.mode = self.apply(mode, cx);
    }

    /// Regulate exactly like the desktop `supply::regulate`: read the rail,
    /// switch CV/CC if needed (one sample of lag) and record the current.
    fn commit_with(&mut self, view: &dyn SolutionView, t: SimTime, _cx: &mut dyn EventCtx) -> Health {
        let Some(plus) = &self.plus_net else {
            return Health::Ok;
        };
        let v = view.value(&format!("v({plus})")).unwrap_or(0.0);
        let s = self.settings;
        if !s.on {
            self.mode = SupplyMode::Off;
            self.amps = 0.0;
            return Health::Ok;
        }
        // The rail was solved for the old set point (no step since the host
        // change): it says nothing about the load at the new one, so keep
        // the mode and the reading until the next step. Without this, a
        // raised set point looked like `(volts - v) / series_r` of overload
        // and the PSU forced its current limit into the load.
        if self.set_at.is_some_and(|at| t <= at) {
            return self.commit(view, t);
        }
        self.set_at = None;
        let i_cv = (s.volts - v) / s.series_r;
        let next = match self.mode {
            SupplyMode::Off | SupplyMode::Cv => {
                if i_cv > s.current_limit {
                    SupplyMode::Cc
                } else {
                    SupplyMode::Cv
                }
            }
            SupplyMode::Cc => {
                // The device holds the rail at or below the set point in CC;
                // back to CV once the CV current would be within the limit.
                if i_cv > s.current_limit {
                    SupplyMode::Cc
                } else {
                    SupplyMode::Cv
                }
            }
        };
        self.mode = next;
        self.amps = match self.mode {
            SupplyMode::Cc => s.current_limit,
            SupplyMode::Cv => i_cv.max(0.0),
            SupplyMode::Off => 0.0,
        };
        self.commit(view, t)
    }

    /// Rating check only; regulation happens in [`Component::commit_with`].
    fn commit(&mut self, view: &dyn SolutionView, _t: SimTime) -> Health {
        let Some(plus) = &self.plus_net else {
            return Health::Ok;
        };
        let v = view.value(&format!("v({plus})")).unwrap_or(0.0);
        if !self.settings.on {
            return Health::Ok;
        }
        match self.settings.rated {
            Some((lo, hi)) if lo > hi => Health::Warning(no_common_range(lo, hi)),
            Some((lo, hi)) if !(lo..=hi).contains(&v) && v > 0.05 => {
                Health::Warning(format!("rail {v:.3} V outside {lo}–{hi} V"))
            }
            _ => Health::Ok,
        }
    }

    fn terminal_currents(&self, view: &dyn SolutionView, out: &mut [f64]) {
        out.fill(0.0);
        if out.len() < 2 {
            return;
        }
        // The device probe `i(name)` is the current delivered out of +.
        let i_out = view.value(&format!("i({})", self.device_name())).unwrap_or(0.0);
        let into_plus = -i_out;
        out[0] = into_plus;
        out[1] = -into_plus;
    }

    fn readings(&self, view: &dyn SolutionView, out: &mut Vec<(String, f64)>) {
        if let Some(plus) = &self.plus_net {
            let v = view.value(&format!("v({plus})")).unwrap_or(0.0);
            out.push(("volts".into(), v));
        }
        let mut i = [0.0; 2];
        self.terminal_currents(view, &mut i);
        out.push(("amps".into(), -i[0]));
        // The regulated current from the last commit, as the PSU face shows it.
        out.push(("regulated_amps".into(), self.amps));
        out.push((
            "mode".into(),
            match self.mode {
                SupplyMode::Off => 0.0,
                SupplyMode::Cv => 1.0,
                SupplyMode::Cc => 2.0,
            },
        ));
        if let Some((lo, hi)) = self.settings.rated {
            out.push(("rated_min".into(), lo));
            out.push(("rated_max".into(), hi));
        }
    }
}

fn net_name(circuit: &of_analog::Circuit, node: of_analog::Var) -> String {
    circuit
        .vars()
        .get(node)
        .map(|v| {
            v.name
                .strip_prefix("v(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("gnd")
                .to_owned()
        })
        .unwrap_or_else(|| "gnd".into())
}

#[cfg(test)]
mod settings_tests {
    use super::*;

    #[test]
    fn warning_when_outside_rated_range() {
        let mut s = SupplySettings {
            volts: 2.5,
            ..SupplySettings::default()
        };
        assert!(!s.set_outside_rated(), "no load rating, no warning");
        assert_eq!(
            no_common_range(4.5, 3.6),
            "parts on this rail have no common supply range (one needs at least 4.5 V, another at most 3.6 V)"
        );
        s.volts = 3.3;
        s.rated = Some((3.0, 3.6));
        assert!(!s.set_outside_rated());
        s.volts = 2.5;
        assert!(s.set_outside_rated());
        s.volts = 5.0;
        assert!(s.set_outside_rated());
        s.on = false;
        assert!(!s.set_outside_rated());
    }

    #[test]
    fn the_face_shows_the_average_when_there_is_one() {
        let mut st = SupplyStatus {
            amps: 0.02,
            ..SupplyStatus::default()
        };
        assert_eq!(st.display_amps(), 0.02);
        st.amps_avg = Some(0.025);
        assert_eq!(st.display_amps(), 0.025);
    }
}
