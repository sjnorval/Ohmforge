//! Bench supply output: a smooth CV/CC characteristic (Newton-friendly).
//!
//! The current out of `p` is the smaller of the constant-voltage current
//! `(v - vd) / r` and the limit, joined by a hyperbola
//! `(a + b - sqrt((a - b)^2 + w^2)) / 2` of width `w` (a millionth of the
//! limit, so the CV and CC branches stay within 1 ppm of their straight
//! lines). In CC the output can never rise above the set point: the
//! current out of `p` is positive only while `vd < v`.

use crate::circuit::{Device, LoadCtx, Setup, Var};
use crate::matrix::Slot;

/// Bench supply output between `p` (+) and `n` (−).
#[derive(Debug, Clone)]
pub struct BenchSupply {
    name: String,
    p: Var,
    n: Var,
    /// Set point (V).
    pub v: f64,
    /// Output resistance in CV (Ω).
    pub r: f64,
    /// Current limit (A).
    pub limit: f64,
    /// Output enabled; off is open (1 TΩ).
    pub on: bool,
    s: [Slot; 4],
}

/// Resistance of a supply that is off (Ω).
const OFF_R: f64 = 1e12;
/// Conductance always across the output (S), so a CC output with nothing
/// else on it still has a DC path.
const G_FLOOR: f64 = 1e-12;

impl BenchSupply {
    /// Supply `name` of `v` volts behind `r` ohms, limited to `limit` amps.
    pub fn new(name: &str, p: Var, n: Var, v: f64, r: f64, limit: f64, on: bool) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            v,
            r: r.max(1e-6),
            limit: limit.max(1e-9),
            on,
            s: [Slot::NONE; 4],
        }
    }

    /// Current out of `p` (A) and its derivative with respect to the
    /// output voltage `vd` (S, never positive), with sources scaled by
    /// `src` (source stepping).
    pub fn current(&self, vd: f64, src: f64) -> (f64, f64) {
        if !self.on {
            return (-vd / OFF_R, -1.0 / OFF_R);
        }
        let a = (self.v * src - vd) / self.r;
        let b = self.limit * src;
        let w = 1e-6 * self.limit;
        let s = ((a - b) * (a - b) + w * w).sqrt();
        let i = 0.5 * (a + b - s);
        let di = -0.5 / self.r * (1.0 - (a - b) / s);
        (i, di)
    }

    /// True when the output is current limited (the CV current would
    /// exceed the limit at `vd`).
    pub fn limited(&self, vd: f64) -> bool {
        self.on && (self.v - vd) / self.r > self.limit
    }
}

impl Device for BenchSupply {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.s = s.slots2(self.p, self.n);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let vd = cx.v(self.p) - cx.v(self.n);
        let (i, di) = self.current(vd, cx.src_factor);
        // Linearised: i(vd') = i - g (vd' - vd) out of p, g = -di.
        let g = -di + G_FLOOR;
        cx.conductance(&self.s, g);
        cx.current(self.n, self.p, i + g * vd);
    }
    /// The CV/CC corner is a kink in the characteristic: report a step that
    /// crosses it so the stepper lands on the corner and restarts its
    /// history there (as at a switch), instead of integrating across it
    /// (trapezoidal overshoot above the set point).
    fn state_crossing(&self, x: &[f64], x_new: &[f64], _states: &crate::integrate::States) -> Option<(f64, f64)> {
        if !self.on {
            return None;
        }
        let f = |x: &[f64]| (self.v - (x[self.p] - x[self.n])) / self.r - self.limit;
        let (f0, f1) = (f(x), f(x_new));
        (f0 * f1 < 0.0).then_some((f0, f1))
    }
    fn set_param(&mut self, name: &str, value: f64) -> bool {
        match name {
            "v" if value.is_finite() => self.v = value,
            "r" if value.is_finite() && value > 0.0 => self.r = value.max(1e-6),
            "limit" if value.is_finite() && value > 0.0 => self.limit = value.max(1e-9),
            "on" if value.is_finite() => self.on = value != 0.0,
            _ => return false,
        }
        true
    }
    fn get_param(&self, name: &str) -> Option<f64> {
        match name {
            "v" => Some(self.v),
            "r" => Some(self.r),
            "limit" => Some(self.limit),
            "on" => Some(if self.on { 1.0 } else { 0.0 }),
            _ => None,
        }
    }
    fn probes(&self, x: &[f64], out: &mut Vec<(String, f64)>) {
        // Current delivered out of the p terminal.
        let vd = x[self.p] - x[self.n];
        out.push((format!("i({})", self.name), self.current(vd, 1.0).0));
    }
}

#[cfg(test)]
mod tests {
    use of_core::{Kernel, SimTime};

    use super::BenchSupply;
    use crate::analysis::Options;
    use crate::circuit::{Circuit, GROUND};
    use crate::devices::{Capacitor, Diode, DiodeModel, Resistor};
    use crate::engine::{AnalogEngine, EngineConfig};

    fn engine(ckt: Circuit, uic: bool) -> AnalogEngine {
        AnalogEngine::new(
            "analog",
            ckt,
            Options::default(),
            EngineConfig {
                max_step: 1e-5,
                record: false,
                uic,
                ..EngineConfig::default()
            },
        )
        .unwrap()
    }

    /// Run `eng` to each of `times` (s), reading `sig` there.
    fn run(eng: AnalogEngine, times: &[f64], sig: &str) -> Vec<f64> {
        let mut k = Kernel::new(SimTime::from_us(10));
        let id = k.add_engine(Box::new(eng));
        times
            .iter()
            .map(|&t| {
                k.run_until(SimTime::from_secs_f64(t)).unwrap();
                k.engine_as::<AnalogEngine>(id).unwrap().value(sig).unwrap()
            })
            .collect()
    }

    /// The runaway case: 10 mA limit into 10 uF (and 100 kohm), from cold.
    /// C1 charges at the limit (1 V/ms) and stops at the set point, never
    /// above it.
    #[test]
    fn cc_charges_at_the_limit_and_never_passes_the_set_point() {
        let mut ckt = Circuit::new();
        let out = ckt.node("out");
        ckt.add(BenchSupply::new("psu", out, GROUND, 5.0, 0.1, 0.01, true))
            .unwrap();
        ckt.add(Capacitor::new("c1", out, GROUND, 1e-5, None)).unwrap();
        ckt.add(Resistor::new("r1", out, GROUND, 1e5)).unwrap();
        let times: Vec<f64> = (1..=200).map(|k| k as f64 * 1e-4).collect();
        let v = run(engine(ckt, true), &times, "v(out)");
        // 1 ms in: 10 mA less the 100 kohm's few uA, 1 V/ms.
        let i_at_1ms = 0.01 - v[9] / 1e5;
        assert!(
            (v[9] - i_at_1ms * 1e-3 / 1e-5).abs() < 1e-3 * v[9],
            "v(1 ms) = {}",
            v[9]
        );
        let vmax = v.iter().copied().fold(f64::MIN, f64::max);
        assert!(vmax <= 5.0, "rail rose to {vmax} V");
        // Settled in CV: 5 V less 50 uA x 0.1 ohm.
        let end = *v.last().unwrap();
        assert!((end - (5.0 - 5e-5 * 0.1)).abs() < 1e-6, "end {end}");
    }

    /// The operating point crosses over from CV to CC as the load grows,
    /// and the current reads the limit in CC.
    #[test]
    fn cv_cc_crossover() {
        for (load, cc) in [(1e3, false), (500.0, false), (499.0, true), (100.0, true), (1.0, true)] {
            let mut ckt = Circuit::new();
            let out = ckt.node("out");
            ckt.add(BenchSupply::new("psu", out, GROUND, 5.0, 0.1, 0.01, true))
                .unwrap();
            ckt.add(Resistor::new("rl", out, GROUND, load)).unwrap();
            let eng = engine(ckt, false);
            let v = eng.value("v(out)").unwrap();
            let i = eng.value("i(psu)").unwrap();
            let (v_want, i_want) = if cc {
                (0.01 * load, 0.01)
            } else {
                (5.0 * load / (load + 0.1), 5.0 / (load + 0.1))
            };
            assert!(
                (v - v_want).abs() <= 1e-9_f64.max(1e-6 * v_want),
                "{load} ohm: v {v} vs {v_want}"
            );
            assert!(
                (i - i_want).abs() <= 1e-9_f64.max(1e-6 * i_want),
                "{load} ohm: i {i} vs {i_want}"
            );
            assert!((i - v / load).abs() < 1e-9, "{load} ohm: KCL");
        }
    }

    /// A cold start straight into forward-biased junctions: an LED with no
    /// resistor on a 10 mA limit, and a diode-fed capacitor. Newton must
    /// converge from all-zero and the LED takes exactly the limit.
    #[test]
    fn cold_start_into_forward_junctions() {
        let mut ckt = Circuit::new();
        let out = ckt.node("out");
        let cap = ckt.node("cap");
        ckt.add(BenchSupply::new("psu", out, GROUND, 5.0, 0.1, 0.01, true))
            .unwrap();
        let mut led = std::collections::BTreeMap::new();
        led.insert("is".to_owned(), 1e-18);
        led.insert("n".to_owned(), 2.0);
        led.insert("rs".to_owned(), 4.0);
        let (led, _) = DiodeModel::from_params(&led);
        ckt.add(Diode::new("d1", out, GROUND, led, 1.0, false)).unwrap();
        let (sig, _) = DiodeModel::from_params(&std::collections::BTreeMap::from([("is".to_owned(), 4e-9)]));
        ckt.add(Diode::new("d2", out, cap, sig, 1.0, false)).unwrap();
        ckt.add(Capacitor::new("c1", cap, GROUND, 1e-7, None)).unwrap();
        let v = run(engine(ckt, true), &[1e-5, 1e-3, 5e-3], "i(psu)");
        // The diode-fed capacitor takes its share early; once it is charged
        // the LED takes the whole limit.
        assert!((v[2] - 0.01).abs() <= 1e-8, "i(psu) {}", v[2]);
        assert!(v.iter().all(|&i| i <= 0.01 + 1e-9), "{v:?}");
    }
}
