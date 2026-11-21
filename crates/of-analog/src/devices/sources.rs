//! Independent and controlled sources.

use crate::circuit::{Device, LoadCtx, Mode, Setup, TranSpec, Var};
use crate::matrix::Slot;
use crate::waveform::Wave;

/// Value specification of an independent source.
#[derive(Debug, Clone, PartialEq)]
pub struct SourceValue {
    /// DC value used for operating points (falls back to the waveform at t = 0).
    pub dc: Option<f64>,
    /// Transient waveform (falls back to `dc`).
    pub wave: Option<Wave>,
}

impl SourceValue {
    /// A constant source.
    pub fn dc(v: f64) -> Self {
        Self {
            dc: Some(v),
            wave: None,
        }
    }

    /// A source with a transient waveform (DC value from the waveform at 0).
    pub fn wave(w: Wave) -> Self {
        Self {
            dc: None,
            wave: Some(w),
        }
    }

    fn at(&self, cx: &LoadCtx<'_>) -> f64 {
        let v = match (cx.mode, &self.wave, self.dc) {
            (Mode::Dc, Some(w), _) if cx.tran_op => w.value(0.0, &cx.tran),
            (Mode::Dc, _, Some(dc)) => dc,
            (Mode::Dc, Some(w), None) => w.value(0.0, &cx.tran),
            (_, Some(w), _) => w.value(cx.time, &cx.tran),
            (_, None, dc) => dc.unwrap_or(0.0),
        };
        v * cx.src_factor
    }

    fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        self.wave.as_ref().and_then(|w| w.next_breakpoint(t, tran))
    }
}

/// `dc` replaces the value (and any waveform) by a constant.
fn set_source_param(v: &mut SourceValue, name: &str, value: f64) -> bool {
    if name == "dc" && value.is_finite() {
        *v = SourceValue::dc(value);
        true
    } else {
        false
    }
}

fn get_source_param(v: &SourceValue, name: &str) -> Option<f64> {
    if name == "dc" { v.dc } else { None }
}

/// A pin driver: a Thevenin source `v` behind `r` between `p` and `n`,
/// changed at run time by the scheduler engine (`set_param("v" | "r")`).
/// A large `r` (e.g. [`PinDriver::HI_Z`]) models a high-impedance output.
#[derive(Debug, Clone)]
pub struct PinDriver {
    name: String,
    p: Var,
    n: Var,
    /// Open-circuit voltage (V).
    pub v: f64,
    /// Output resistance (Ω).
    pub r: f64,
    s: [Slot; 4],
}

impl PinDriver {
    /// Output resistance used for a disabled (high-impedance) driver.
    pub const HI_Z: f64 = 1e12;

    /// Driver of `v` volts behind `r` ohms from `p` to `n`.
    pub fn new(name: &str, p: Var, n: Var, v: f64, r: f64) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            v,
            r: r.max(1e-6),
            s: [Slot::NONE; 4],
        }
    }
}

impl Device for PinDriver {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.s = s.slots2(self.p, self.n);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let g = 1.0 / self.r;
        cx.conductance(&self.s, g);
        // Norton equivalent: v * g flows into p.
        cx.current(self.n, self.p, self.v * g * cx.src_factor);
    }
    fn set_param(&mut self, name: &str, value: f64) -> bool {
        match name {
            "v" if value.is_finite() => self.v = value,
            "r" if value.is_finite() && value > 0.0 => self.r = value.max(1e-6),
            _ => return false,
        }
        true
    }
    fn get_param(&self, name: &str) -> Option<f64> {
        match name {
            "v" => Some(self.v),
            "r" => Some(self.r),
            _ => None,
        }
    }
    fn probes(&self, x: &[f64], out: &mut Vec<(String, f64)>) {
        // Current delivered out of the p terminal.
        out.push((format!("i({})", self.name), (self.v - (x[self.p] - x[self.n])) / self.r));
    }
}

/// Independent voltage source; branch current `i(name)` flows from `p`
/// through the source to `n` (SPICE convention).
#[derive(Debug, Clone)]
pub struct VoltageSource {
    name: String,
    p: Var,
    n: Var,
    /// Value.
    pub value: SourceValue,
    /// Small-signal magnitude and phase (degrees) for `.ac`.
    pub ac: Option<(f64, f64)>,
    b: Var,
    s: [Slot; 4],
}

impl VoltageSource {
    /// Source with `p` positive.
    pub fn new(name: &str, p: Var, n: Var, value: SourceValue) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            value,
            ac: None,
            b: 0,
            s: [Slot::NONE; 4],
        }
    }

    /// Set the `.ac` excitation (magnitude, phase in degrees).
    pub fn with_ac(mut self, ac: Option<(f64, f64)>) -> Self {
        self.ac = ac;
        self
    }
}

impl Device for VoltageSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        0
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.b = s.branch(&self.name);
        self.s = [
            s.slot(self.p, self.b),
            s.slot(self.n, self.b),
            s.slot(self.b, self.p),
            s.slot(self.b, self.n),
        ];
    }
    fn branch(&self) -> Option<Var> {
        Some(self.b)
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        cx.add(self.s[0], 1.0);
        cx.add(self.s[1], -1.0);
        cx.add(self.s[2], 1.0);
        cx.add(self.s[3], -1.0);
        let v = self.value.at(cx);
        cx.rhs(self.b, v);
    }
    fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        self.value.next_breakpoint(t, tran)
    }
    fn is_changing(&self, t: f64, tran: &TranSpec) -> bool {
        self.value.wave.as_ref().is_some_and(|w| w.is_changing(t, tran))
    }
    fn set_dc(&mut self, value: f64) -> bool {
        self.value.dc = Some(value);
        true
    }
    fn set_param(&mut self, name: &str, value: f64) -> bool {
        set_source_param(&mut self.value, name, value)
    }
    fn get_param(&self, name: &str) -> Option<f64> {
        get_source_param(&self.value, name)
    }
    fn ac_stimulus(&self, re: &mut [f64], im: &mut [f64]) {
        if let Some((mag, ph)) = self.ac {
            let (s, c) = ph.to_radians().sin_cos();
            re[self.b] += mag * c;
            im[self.b] += mag * s;
        }
    }
}

/// Independent current source: positive current flows from `p` through
/// the source to `n` (out of node `p`, into node `n`).
#[derive(Debug, Clone)]
pub struct CurrentSource {
    name: String,
    p: Var,
    n: Var,
    /// Value.
    pub value: SourceValue,
    /// Small-signal magnitude and phase (degrees) for `.ac`.
    pub ac: Option<(f64, f64)>,
}

impl CurrentSource {
    /// Current source from `p` to `n`.
    pub fn new(name: &str, p: Var, n: Var, value: SourceValue) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            value,
            ac: None,
        }
    }

    /// Set the `.ac` excitation (magnitude, phase in degrees).
    pub fn with_ac(mut self, ac: Option<(f64, f64)>) -> Self {
        self.ac = ac;
        self
    }
}

impl Device for CurrentSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, _s: &mut Setup<'_>) {}
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let i = self.value.at(cx);
        cx.current(self.p, self.n, i);
    }
    fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        self.value.next_breakpoint(t, tran)
    }
    fn is_changing(&self, t: f64, tran: &TranSpec) -> bool {
        self.value.wave.as_ref().is_some_and(|w| w.is_changing(t, tran))
    }
    fn set_dc(&mut self, value: f64) -> bool {
        self.value.dc = Some(value);
        true
    }
    fn set_param(&mut self, name: &str, value: f64) -> bool {
        set_source_param(&mut self.value, name, value)
    }
    fn get_param(&self, name: &str) -> Option<f64> {
        get_source_param(&self.value, name)
    }
    fn ac_stimulus(&self, re: &mut [f64], im: &mut [f64]) {
        if let Some((mag, ph)) = self.ac {
            let (s, c) = ph.to_radians().sin_cos();
            re[self.p] -= mag * c;
            im[self.p] -= mag * s;
            re[self.n] += mag * c;
            im[self.n] += mag * s;
        }
    }
}

/// Which linear controlled source.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    /// E: `v(p,n) = gain * v(cp,cn)`.
    Vcvs {
        /// Controlling positive node.
        cp: Var,
        /// Controlling negative node.
        cn: Var,
    },
    /// G: current `gain * v(cp,cn)` from `p` to `n`.
    Vccs {
        /// Controlling positive node.
        cp: Var,
        /// Controlling negative node.
        cn: Var,
    },
    /// F: current `gain * i(source)` from `p` to `n`.
    Cccs {
        /// Controlling voltage source name.
        source: String,
    },
    /// H: `v(p,n) = gain * i(source)`.
    Ccvs {
        /// Controlling voltage source name.
        source: String,
    },
}

/// Linear controlled source (E, G, F, H).
#[derive(Debug, Clone)]
pub struct ControlledSource {
    name: String,
    p: Var,
    n: Var,
    /// Gain, transconductance or transresistance.
    pub gain: f64,
    /// Kind and controlling quantity.
    pub control: Control,
    b: Var,
    s: Vec<(Slot, f64)>,
    /// Set if the controlling source could not be found at setup.
    pub missing_control: bool,
}

impl ControlledSource {
    /// New controlled source.
    pub fn new(name: &str, p: Var, n: Var, gain: f64, control: Control) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            gain,
            control,
            b: 0,
            s: Vec::new(),
            missing_control: false,
        }
    }
}

impl Device for ControlledSource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        match self.control {
            Control::Vcvs { .. } => 0,
            _ => 2,
        }
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        let (p, n, g) = (self.p, self.n, self.gain);
        let mut entries = Vec::new();
        let ctrl_branch = |s: &Setup<'_>, src: &str| s.branch_of(src);
        match &self.control {
            Control::Vcvs { cp, cn } => {
                self.b = s.branch(&self.name);
                let b = self.b;
                entries.extend([
                    (s.slot(p, b), 1.0),
                    (s.slot(n, b), -1.0),
                    (s.slot(b, p), 1.0),
                    (s.slot(b, n), -1.0),
                ]);
                entries.extend([(s.slot(b, *cp), -g), (s.slot(b, *cn), g)]);
            }
            Control::Vccs { cp, cn } => {
                entries.extend([
                    (s.slot(p, *cp), g),
                    (s.slot(p, *cn), -g),
                    (s.slot(n, *cp), -g),
                    (s.slot(n, *cn), g),
                ]);
            }
            Control::Cccs { source } => match ctrl_branch(s, source) {
                Some(bc) => entries.extend([(s.slot(p, bc), g), (s.slot(n, bc), -g)]),
                None => self.missing_control = true,
            },
            Control::Ccvs { source } => {
                self.b = s.branch(&self.name);
                let b = self.b;
                entries.extend([
                    (s.slot(p, b), 1.0),
                    (s.slot(n, b), -1.0),
                    (s.slot(b, p), 1.0),
                    (s.slot(b, n), -1.0),
                ]);
                match ctrl_branch(s, source) {
                    Some(bc) => entries.push((s.slot(b, bc), -g)),
                    None => self.missing_control = true,
                }
            }
        }
        self.s = entries;
    }
    fn branch(&self) -> Option<Var> {
        matches!(self.control, Control::Vcvs { .. } | Control::Ccvs { .. }).then_some(self.b)
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        for &(slot, v) in &self.s {
            cx.add(slot, v);
        }
    }
}

/// Output of a polynomial source.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PolyKind {
    /// E: `v(p,n) = f(x)`.
    Voltage,
    /// G: current `f(x)` from `p` through the source to `n`.
    Current,
}

/// SPICE2 `POLY(n)` source:
/// `f(x) = p0 + p1 x1 + ... + pn xn + p(n+1) x1^2 + p(n+2) x1 x2 + ...`
/// (terms by degree, then in lexicographic order of the variable indices),
/// where `xk` are controlling voltages. Used, for
/// example, to multiply a voltage by a current-sense voltage to get the
/// power that heats a thermal node.
#[derive(Debug, Clone)]
pub struct PolySource {
    name: String,
    p: Var,
    n: Var,
    kind: PolyKind,
    ctrl: Vec<(Var, Var)>,
    coeffs: Vec<f64>,
    /// Exponent of every variable in each term.
    terms: Vec<Vec<u32>>,
    b: Var,
    s_out: [Slot; 4],
    s_ctrl: Vec<[Slot; 4]>,
}

/// Exponent vectors of the first `count` POLY terms in `dim` variables.
fn poly_terms(dim: usize, count: usize) -> Vec<Vec<u32>> {
    let mut out = vec![vec![0; dim]];
    let mut degree = 1;
    while out.len() < count {
        // Non-decreasing index sequences of length `degree`, in order.
        let mut idx = vec![0usize; degree];
        loop {
            let mut e = vec![0u32; dim];
            for &i in &idx {
                e[i] += 1;
            }
            out.push(e);
            // Next non-decreasing sequence.
            let mut k = degree;
            while k > 0 && idx[k - 1] == dim - 1 {
                k -= 1;
            }
            if k == 0 {
                break;
            }
            idx[k - 1] += 1;
            let v = idx[k - 1];
            idx[k..].iter_mut().for_each(|x| *x = v);
        }
        degree += 1;
    }
    out.truncate(count);
    out
}

impl PolySource {
    /// Polynomial source with controlling node pairs `ctrl` and SPICE
    /// coefficient order.
    pub fn new(name: &str, p: Var, n: Var, kind: PolyKind, ctrl: Vec<(Var, Var)>, coeffs: Vec<f64>) -> Self {
        let terms = poly_terms(ctrl.len().max(1), coeffs.len());
        Self {
            name: name.into(),
            p,
            n,
            kind,
            ctrl,
            coeffs,
            terms,
            b: 0,
            s_out: [Slot::NONE; 4],
            s_ctrl: Vec::new(),
        }
    }

    /// Value and gradient at controlling voltages `xs`.
    fn eval(&self, xs: &[f64]) -> (f64, Vec<f64>) {
        let mut f = 0.0;
        let mut grad = vec![0.0; xs.len()];
        for (c, e) in self.coeffs.iter().zip(&self.terms) {
            if *c == 0.0 {
                continue;
            }
            let mut term = *c;
            for (x, &k) in xs.iter().zip(e) {
                term *= x.powi(k as i32);
            }
            f += term;
            for j in 0..xs.len() {
                if e[j] == 0 {
                    continue;
                }
                let mut d = *c * f64::from(e[j]) * xs[j].powi(e[j] as i32 - 1);
                for (i, (x, &k)) in xs.iter().zip(e).enumerate() {
                    if i != j {
                        d *= x.powi(k as i32);
                    }
                }
                grad[j] += d;
            }
        }
        (f, grad)
    }
}

impl Device for PolySource {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        match self.kind {
            PolyKind::Voltage => 0,
            PolyKind::Current => 1,
        }
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        let (p, n) = (self.p, self.n);
        let row = match self.kind {
            PolyKind::Voltage => {
                self.b = s.branch(&self.name);
                let b = self.b;
                self.s_out = [s.slot(p, b), s.slot(n, b), s.slot(b, p), s.slot(b, n)];
                [b, b]
            }
            PolyKind::Current => [p, n],
        };
        self.s_ctrl = self
            .ctrl
            .iter()
            .map(|&(cp, cn)| {
                [
                    s.slot(row[0], cp),
                    s.slot(row[0], cn),
                    s.slot(row[1], cp),
                    s.slot(row[1], cn),
                ]
            })
            .collect();
    }
    fn branch(&self) -> Option<Var> {
        (self.kind == PolyKind::Voltage).then_some(self.b)
    }
    fn is_nonlinear(&self) -> bool {
        self.terms.iter().any(|e| e.iter().sum::<u32>() > 1)
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let xs: Vec<f64> = self.ctrl.iter().map(|&(cp, cn)| cx.v(cp) - cx.v(cn)).collect();
        let (f, grad) = self.eval(&xs);
        // Linearised: out = f0 + sum g_k (x_k - x0_k).
        let i_eq = f - grad.iter().zip(&xs).map(|(g, x)| g * x).sum::<f64>();
        match self.kind {
            PolyKind::Voltage => {
                cx.add(self.s_out[0], 1.0);
                cx.add(self.s_out[1], -1.0);
                cx.add(self.s_out[2], 1.0);
                cx.add(self.s_out[3], -1.0);
                for (s, g) in self.s_ctrl.iter().zip(&grad) {
                    cx.add(s[0], -g);
                    cx.add(s[1], *g);
                }
                cx.rhs(self.b, i_eq);
            }
            PolyKind::Current => {
                for (s, g) in self.s_ctrl.iter().zip(&grad) {
                    cx.add(s[0], *g);
                    cx.add(s[1], -g);
                    cx.add(s[2], -g);
                    cx.add(s[3], *g);
                }
                cx.current(self.p, self.n, i_eq);
            }
        }
    }
}

#[cfg(test)]
mod poly_tests {
    use super::poly_terms;

    #[test]
    fn spice_term_order() {
        let t = poly_terms(2, 10);
        let want: [[u32; 2]; 10] = [
            [0, 0],
            [1, 0],
            [0, 1],
            [2, 0],
            [1, 1],
            [0, 2],
            [3, 0],
            [2, 1],
            [1, 2],
            [0, 3],
        ];
        assert_eq!(t, want.map(|w| w.to_vec()).to_vec());
        assert_eq!(poly_terms(1, 4), vec![vec![0], vec![1], vec![2], vec![3]]);
        assert_eq!(poly_terms(3, 5)[3..], [vec![0, 0, 1], vec![2, 0, 0]]);
    }
}
