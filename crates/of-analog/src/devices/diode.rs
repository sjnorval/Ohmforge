//! Junction diode: Shockley equation with emission coefficient, series
//! resistance, reverse breakdown, depletion and diffusion capacitance,
//! SPICE3 temperature scaling and junction-voltage limiting. The same
//! device, with optical outputs, models an LED ([`LedParams`]).

use std::collections::BTreeMap;

use super::limit::{REFTEMP, depletion, junction, junction_temp, pnjlim, vcrit, vt_at};
use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Planck constant times the speed of light over the electron charge
/// (V·m): photon energy in eV is this over the wavelength in metres.
const HC_OVER_Q: f64 = 1.239_841_98e-6;

/// Diode model parameters (SPICE names).
#[derive(Debug, Clone, PartialEq)]
pub struct DiodeModel {
    /// Saturation current (A).
    pub is: f64,
    /// Emission coefficient.
    pub n: f64,
    /// Series resistance (Ω).
    pub rs: f64,
    /// Zero-bias junction capacitance (F).
    pub cjo: f64,
    /// Junction potential (V).
    pub vj: f64,
    /// Grading coefficient.
    pub m: f64,
    /// Forward-bias depletion capacitance coefficient.
    pub fc: f64,
    /// Transit time (s).
    pub tt: f64,
    /// Reverse breakdown voltage (V), if any.
    pub bv: Option<f64>,
    /// Current at breakdown voltage (A).
    pub ibv: f64,
    /// Breakdown emission coefficient (defaults to N).
    pub nbv: Option<f64>,
    /// Breakdown-voltage temperature coefficient (V/K).
    pub tcv: f64,
    /// Activation energy for IS (eV).
    pub eg: f64,
    /// IS temperature exponent.
    pub xti: f64,
    /// Nominal temperature (K).
    pub tnom: f64,
    /// Optical parameters when the model is an LED.
    pub led: Option<LedParams>,
}

/// Optical parameters of an LED model (`.model name led(...)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LedParams {
    /// Rated forward current (A); `bright(name)` is `I / IF`.
    pub if_rated: f64,
    /// Peak emission wavelength (m).
    pub wavelength: f64,
    /// External quantum efficiency (photons out per electron).
    pub eqe: f64,
}

impl Default for DiodeModel {
    fn default() -> Self {
        Self {
            is: 1e-14,
            n: 1.0,
            rs: 0.0,
            cjo: 0.0,
            vj: 1.0,
            m: 0.5,
            fc: 0.5,
            tt: 0.0,
            bv: None,
            ibv: 1e-3,
            nbv: None,
            tcv: 0.0,
            eg: 1.11,
            xti: 3.0,
            tnom: REFTEMP,
            led: None,
        }
    }
}

impl DiodeModel {
    /// Build from `.model` parameters; unknown names are returned.
    /// `tnom` is in °C, as in SPICE.
    pub fn from_params(p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        const KNOWN: &[&str] = &[
            "is", "n", "rs", "cjo", "cj0", "cj", "vj", "pb", "m", "mj", "fc", "tt", "bv", "ibv", "nbv", "tcv", "eg",
            "xti", "tnom", "level",
        ];
        let d = Self::default();
        let unknown = p.keys().filter(|k| !KNOWN.contains(&k.as_str())).cloned().collect();
        let m = Self {
            is: param(p, &["is"], d.is),
            n: param(p, &["n"], d.n),
            rs: param(p, &["rs"], d.rs),
            cjo: param(p, &["cjo", "cj0", "cj"], d.cjo),
            vj: param(p, &["vj", "pb"], d.vj),
            m: param(p, &["m", "mj"], d.m),
            fc: param(p, &["fc"], d.fc),
            tt: param(p, &["tt"], d.tt),
            bv: p.get("bv").copied(),
            ibv: param(p, &["ibv"], d.ibv),
            nbv: p.get("nbv").copied(),
            tcv: param(p, &["tcv"], d.tcv),
            eg: param(p, &["eg"], d.eg),
            xti: param(p, &["xti"], d.xti),
            tnom: p.get("tnom").map_or(d.tnom, |c| c + 273.15),
            led: None,
        };
        (m, unknown)
    }

    /// LED model from `.model name led(...)`: diode parameters plus `vf`
    /// (forward voltage at `if`, used to derive IS when IS is not given),
    /// `if` (default 20 mA), `n` (default 2), `wavelength` (default 625 nm,
    /// red) and `eqe` (default 0.1). Also `colour`/`color` as a number
    /// selects a wavelength in nm.
    pub fn led_from_params(p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let mut diode_params = p.clone();
        let mut extra = Vec::new();
        for k in ["vf", "if", "wavelength", "lambda", "eqe", "colour", "color"] {
            if diode_params.remove(k).is_some() {
                extra.push(k);
            }
        }
        diode_params.entry("n".into()).or_insert(2.0);
        let (mut m, unknown) = Self::from_params(&diode_params);
        let if_rated = p.get("if").copied().unwrap_or(20e-3);
        let wl = p
            .get("wavelength")
            .or_else(|| p.get("lambda"))
            .copied()
            .or_else(|| p.get("colour").or_else(|| p.get("color")).map(|nm| nm * 1e-9))
            .unwrap_or(625e-9);
        if !p.contains_key("is") {
            let vf = p.get("vf").copied().unwrap_or_else(|| default_vf(wl));
            let vj = vf - if_rated * m.rs;
            m.is = if_rated / ((vj / (m.n * vt_at(m.tnom))).exp() - 1.0);
        }
        m.led = Some(LedParams {
            if_rated,
            wavelength: wl,
            eqe: p.get("eqe").copied().unwrap_or(0.1),
        });
        (m, unknown)
    }
}

/// Typical forward voltage of an LED of wavelength `wl` (m): roughly the
/// photon energy plus 0.2 V, clamped to 1.6..3.4 V.
fn default_vf(wl: f64) -> f64 {
    (HC_OVER_Q / wl + 0.2).clamp(1.6, 3.4)
}

/// A diode instance.
#[derive(Debug, Clone)]
pub struct Diode {
    name: String,
    a: Var,
    k: Var,
    ai: Var,
    model: DiodeModel,
    area: f64,
    off: bool,
    /// Temperature-scaled saturation current (area included).
    is: f64,
    vte: f64,
    vtebrk: f64,
    vj: f64,
    cj0: f64,
    vcrit: f64,
    xbv: f64,
    vd_old: f64,
    s_rs: [Slot; 4],
    s_j: [Slot; 4],
    state: usize,
}

impl Diode {
    /// Diode from anode `a` to cathode `k`, at the nominal temperature.
    pub fn new(name: &str, a: Var, k: Var, model: DiodeModel, area: f64, off: bool) -> Self {
        let t = model.tnom;
        let mut d = Self {
            name: name.into(),
            a,
            k,
            ai: a,
            model,
            area,
            off,
            is: 0.0,
            vte: 0.0,
            vtebrk: 0.0,
            vj: 0.0,
            cj0: 0.0,
            vcrit: 0.0,
            xbv: 0.0,
            vd_old: 0.0,
            s_rs: [Slot::NONE; 4],
            s_j: [Slot::NONE; 4],
            state: 0,
        };
        d.set_temp(t);
        d
    }

    /// Evaluate at device temperature `temp` (K).
    pub fn with_temp(mut self, temp: f64) -> Self {
        self.set_temp(temp);
        self
    }

    fn set_temp(&mut self, temp: f64) {
        let m = &self.model;
        let vt = vt_at(temp);
        let ratio = temp / m.tnom;
        self.vte = m.n * vt;
        self.vtebrk = m.nbv.unwrap_or(m.n) * vt;
        self.is = m.is * self.area * ((ratio - 1.0) * m.eg / self.vte + m.xti / m.n * ratio.ln()).exp();
        let (vj, cj) = junction_temp(m.vj, m.cjo, m.m, m.tnom, temp);
        self.vj = vj;
        self.cj0 = cj * self.area;
        self.vcrit = vcrit(self.vte, self.is);
        self.xbv = m.bv.map_or(f64::INFINITY, |bv| {
            breakdown_knee(bv - m.tcv * (temp - m.tnom), m.ibv * self.area, self.is, self.vtebrk)
        });
    }

    /// DC current and conductance at junction voltage `vd` (gmin included).
    pub fn dc(&self, vd: f64, gmin: f64) -> (f64, f64) {
        let is = self.is;
        let (i, g) = if vd < -self.xbv {
            let e = (-(self.xbv + vd) / self.vtebrk).min(80.0).exp();
            (-is * e, is * e / self.vtebrk)
        } else {
            junction(vd, is, self.vte)
        };
        (i + gmin * vd, g + gmin)
    }

    /// Device current at solution `x` (anode to cathode).
    fn current_at(&self, x: &[f64]) -> f64 {
        if self.model.rs > 0.0 {
            (x[self.a] - x[self.ai]) * self.area / self.model.rs
        } else {
            self.dc(x[self.a] - x[self.k], 0.0).0
        }
    }
}

/// SPICE3 breakdown-knee adjustment so that the current at `-bv` is `ibv`.
fn breakdown_knee(bv: f64, ibv: f64, is: f64, vt: f64) -> f64 {
    if ibv < is * bv / vt {
        return bv;
    }
    let cbv = ibv;
    let tol = 1e-3 * cbv;
    let mut xbv = bv - vt * (1.0 + cbv / is).ln();
    for _ in 0..25 {
        xbv = bv - vt * (cbv / is + 1.0 - xbv / vt).ln();
        let xcbv = is * ((bv - xbv) / vt).exp() - is + is * xbv / vt;
        if (xcbv - cbv).abs() <= tol {
            break;
        }
    }
    xbv
}

impl Device for Diode {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_nonlinear(&self) -> bool {
        true
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        if self.model.rs > 0.0 {
            self.ai = s.internal_node(&self.name, "a");
            self.s_rs = s.slots2(self.a, self.ai);
        }
        self.s_j = s.slots2(self.ai, self.k);
        self.state = s.states(2);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let mut vd = cx.v(self.ai) - cx.v(self.k);
        if cx.init_junctions {
            vd = if self.off { 0.0 } else { self.vcrit };
        } else if cx.small_signal {
            // Linearising about the converged point: no limiting.
        } else if self.xbv.is_finite() && vd < (-self.xbv + 10.0 * self.vtebrk).min(0.0) {
            // Limit in the breakdown region by mirroring about -BV (SPICE3).
            let (v, lim) = pnjlim(-(vd + self.xbv), -(self.vd_old + self.xbv), self.vtebrk, self.vcrit);
            vd = -(v + self.xbv);
            cx.limited |= lim;
        } else {
            let (v, lim) = pnjlim(vd, self.vd_old, self.vte, self.vcrit);
            vd = v;
            cx.limited |= lim;
        }
        self.vd_old = vd;
        let (mut id, mut gd) = self.dc(vd, cx.gmin);
        if cx.mode != Mode::Dc {
            let m = &self.model;
            let (qj, cj) = depletion(vd, self.cj0, self.vj, m.m, m.fc);
            let (idd, gdd) = self.dc(vd, 0.0);
            cx.states.now[self.state] = m.tt * idd + qj;
            if cx.mode == Mode::Tran {
                let (geq, iq) = cx.integrate(self.state, m.tt * gdd + cj);
                id += iq;
                gd += geq;
            } else {
                cx.states.now[self.state + 1] = 0.0;
            }
        }
        if self.model.rs > 0.0 {
            cx.conductance(&self.s_rs, self.area / self.model.rs);
        }
        cx.conductance(&self.s_j, gd);
        cx.current(self.ai, self.k, id - gd * vd);
    }
    fn probes(&self, x: &[f64], out: &mut Vec<(String, f64)>) {
        // Anode-to-cathode current through the diode at this solution (E5c):
        // the series-resistance current when `rs > 0`, else the junction current.
        let id = self.current_at(x);
        out.push((format!("i({})", self.name), id));
        if let Some(led) = self.model.led {
            let i = id.max(0.0);
            out.push((format!("popt({})", self.name), led.eqe * i * HC_OVER_Q / led.wavelength));
            out.push((format!("bright({})", self.name), i / led.if_rated));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probes_report_the_diode_current() {
        // E5c: `i(name)` is the junction (Shockley) current at the solution,
        // for plain diodes and LEDs alike.
        let d = Diode::new("d1", 1, 0, DiodeModel::default(), 1.0, false);
        let mut out = Vec::new();
        d.probes(&[0.0, 0.6], &mut out);
        let i = out.iter().find(|(n, _)| n == "i(d1)").expect("i(d1) probe").1;
        assert_eq!(i, d.dc(0.6, 0.0).0);
    }

    #[test]
    fn shockley_and_reverse_regions() {
        let d = Diode::new("d1", 1, 0, DiodeModel::default(), 1.0, false);
        let vt = super::super::limit::thermal_voltage();
        let (i, g) = d.dc(0.6, 0.0);
        let exact = 1e-14 * ((0.6 / vt).exp() - 1.0);
        assert!((i - exact).abs() < 1e-12 * exact);
        assert!((g - 1e-14 * (0.6 / vt).exp() / vt).abs() < 1e-9 * g);
        let (ir, _) = d.dc(-5.0, 0.0);
        assert!((ir + 1e-14).abs() < 1e-16);
    }

    #[test]
    fn breakdown_matches_ibv() {
        let model = DiodeModel {
            bv: Some(5.1),
            ibv: 1e-3,
            ..DiodeModel::default()
        };
        let d = Diode::new("dz", 1, 0, model, 1.0, false);
        let (i, _) = d.dc(-5.1, 0.0);
        assert!((i + 1e-3).abs() < 2e-5, "{i}");
    }

    #[test]
    fn forward_voltage_falls_about_2mv_per_kelvin() {
        let m = DiodeModel {
            is: 1e-14,
            ..DiodeModel::default()
        };
        let vf = |t: f64| {
            let d = Diode::new("d", 1, 0, m.clone(), 1.0, false).with_temp(t);
            // Solve dc(v) = 1 mA by bisection.
            let (mut lo, mut hi) = (0.0, 1.5);
            for _ in 0..80 {
                let mid = 0.5 * (lo + hi);
                if d.dc(mid, 0.0).0 > 1e-3 { hi = mid } else { lo = mid }
            }
            lo
        };
        let slope = (vf(REFTEMP + 50.0) - vf(REFTEMP)) / 50.0;
        assert!(slope < -1.5e-3 && slope > -2.5e-3, "{slope}");
    }

    #[test]
    fn led_derives_is_from_vf_and_reports_optical_power() {
        let p: BTreeMap<String, f64> = [
            ("vf".to_string(), 2.0),
            ("if".to_string(), 20e-3),
            ("wavelength".to_string(), 620e-9),
        ]
        .into_iter()
        .collect();
        let (m, unknown) = DiodeModel::led_from_params(&p);
        assert!(unknown.is_empty());
        let d = Diode::new("d1", 1, 0, m, 1.0, false);
        let (i, _) = d.dc(2.0, 0.0);
        assert!((i - 20e-3).abs() < 1e-9, "{i}");
        let mut out = Vec::new();
        d.probes(&[0.0, 2.0], &mut out);
        let get = |k: &str| out.iter().find(|(n, _)| n == k).map(|(_, v)| *v).unwrap();
        let popt = get("popt(d1)");
        let expected = 0.1 * 20e-3 * HC_OVER_Q / 620e-9;
        assert!((popt - expected).abs() < 1e-9 * expected, "{popt}");
        assert!((get("bright(d1)") - 1.0).abs() < 1e-6);
    }
}
