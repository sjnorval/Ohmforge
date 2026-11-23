//! Bench PSU as a [`Component`] (RFC 0001 / design §3d).

use of_analog::devices::{CurrentSource, PinDriver, SourceValue};
use of_component_api::{
    Component, ComponentEvent, ComponentInfo, ElabCtx, ElabError, EventCtx, Health, PinKind, PinSpec, SolutionView,
};
use of_core::SimTime;

/// Default set voltage (V).
pub const DEFAULT_VOLTS: f64 = 3.3;
/// Default series resistance (Ω).
pub const DEFAULT_SERIES_R: f64 = 0.1;
/// Default current limit (A).
pub const DEFAULT_LIMIT_A: f64 = 0.5;
/// ESP32-C3 recommended minimum (V).
pub const RATED_MIN_V: f64 = 3.0;
/// ESP32-C3 recommended maximum (V).
pub const RATED_MAX_V: f64 = 3.6;

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

/// Bench PSU component with explicit `+` and `−` terminals.
pub struct BenchPsu {
    info: ComponentInfo,
    settings: SupplySettings,
    mode: SupplyMode,
    /// Device name prefix (refdes lowercased).
    device: String,
    plus_net: Option<String>,
    minus_net: Option<String>,
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
        }
    }

    fn thevenin_name(&self) -> String {
        self.device.clone()
    }

    fn cc_name(&self) -> String {
        format!("{}_cc", self.device)
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
        self.settings = self.settings.sanitized();

        let s = self.settings;
        let (v, r) = if s.on {
            (s.volts, s.series_r)
        } else {
            (0.0, PinDriver::HI_Z)
        };
        let name = self.thevenin_name();
        cx.add_device(Box::new(PinDriver::new(&name, plus, minus, v, r)))?;
        cx.add_device(Box::new(CurrentSource::new(
            &self.cc_name(),
            minus,
            plus,
            SourceValue::dc(0.0),
        )))?;
        Ok(())
    }

    fn on_event(&mut self, ev: &ComponentEvent, _cx: &mut dyn EventCtx) {
        let ComponentEvent::HostParam { name, value } = ev else {
            return;
        };
        match name.as_str() {
            "volts" => self.settings.volts = *value,
            "series_r" => self.settings.series_r = *value,
            "current_limit" => self.settings.current_limit = *value,
            "on" => self.settings.on = *value != 0.0,
            _ => {}
        }
        self.settings = self.settings.sanitized();
    }

    fn commit(&mut self, view: &dyn SolutionView, _t: SimTime) -> Health {
        let Some(plus) = &self.plus_net else {
            return Health::Ok;
        };
        let v = view.value(&format!("v({plus})")).unwrap_or(0.0);
        let s = self.settings;
        if !s.on {
            self.mode = SupplyMode::Off;
            return Health::Ok;
        }
        let i_cv = (s.volts - v) / s.series_r;
        self.mode = if i_cv > s.current_limit {
            SupplyMode::Cc
        } else {
            SupplyMode::Cv
        };
        if !(RATED_MIN_V..=RATED_MAX_V).contains(&v) && v > 0.05 {
            Health::Warning(format!("rail {v:.3} V outside {RATED_MIN_V}–{RATED_MAX_V} V"))
        } else {
            Health::Ok
        }
    }

    fn terminal_currents(&self, view: &dyn SolutionView, out: &mut [f64]) {
        out.fill(0.0);
        if out.len() < 2 {
            return;
        }
        // Current out of + equals −i(thevenin) in SPICE sign (into + terminal of source).
        // PinDriver branch current i(name) flows into the + terminal of the Thevenin source.
        let name = self.thevenin_name();
        // PinDriver probe `i(name)` is current delivered *out of* the + terminal.
        let i_out = view.value(&format!("i({name})")).unwrap_or(0.0);
        let i_cc = view.value(&format!("i({})", self.cc_name())).unwrap_or(0.0);
        // Current *into* + is the opposite of current out (CC source adds into +).
        let into_plus = -i_out + i_cc;
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
        out.push((
            "mode".into(),
            match self.mode {
                SupplyMode::Off => 0.0,
                SupplyMode::Cv => 1.0,
                SupplyMode::Cc => 2.0,
            },
        ));
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
