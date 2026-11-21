//! Resistor, capacitor (with optional ESR/ESL), inductor (with optional
//! series resistance) and mutual inductance between two inductors.
//!
//! Temperature coefficients (`tc1`, `tc2`) are applied to the value by the
//! netlist builder; tolerances are stored for later Monte Carlo runs.

use crate::circuit::{Device, GROUND, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Linear resistor.
#[derive(Debug, Clone)]
pub struct Resistor {
    name: String,
    p: Var,
    n: Var,
    /// Resistance in ohms.
    pub ohms: f64,
    /// Relative tolerance (e.g. 0.01 for 1 %), for Monte Carlo analyses.
    pub tolerance: Option<f64>,
    s: [Slot; 4],
}

impl Resistor {
    /// Resistor of `ohms` between `p` and `n`.
    pub fn new(name: &str, p: Var, n: Var, ohms: f64) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            ohms,
            tolerance: None,
            s: [Slot::NONE; 4],
        }
    }

    /// Record a relative tolerance.
    pub fn with_tolerance(mut self, tol: Option<f64>) -> Self {
        self.tolerance = tol;
        self
    }
}

impl Device for Resistor {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.s = s.slots2(self.p, self.n);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        cx.conductance(&self.s, 1.0 / self.ohms);
    }
    fn set_param(&mut self, name: &str, value: f64) -> bool {
        if name == "r" && value.is_finite() && value != 0.0 {
            self.ohms = value;
            true
        } else {
            false
        }
    }
    fn get_param(&self, name: &str) -> Option<f64> {
        if name == "r" { Some(self.ohms) } else { None }
    }
}

/// Linear capacitor with optional initial voltage (used with `uic`).
#[derive(Debug, Clone)]
pub struct Capacitor {
    name: String,
    p: Var,
    n: Var,
    /// Capacitance in farads.
    pub farads: f64,
    /// Initial voltage for `uic` runs.
    pub ic: Option<f64>,
    /// Relative tolerance, for Monte Carlo analyses.
    pub tolerance: Option<f64>,
    /// Equivalent series resistance (Ω), 0 for none.
    pub esr: f64,
    /// Equivalent series inductance (H), 0 for none.
    pub esl: f64,
    /// Node of the ideal capacitor's positive plate.
    cp: Var,
    s: [Slot; 4],
    s_esr: [Slot; 4],
    esl_branch: Option<SeriesL>,
    state: usize,
}

/// Branch stamps of an internal series inductance from `p` to `n`.
#[derive(Debug, Clone, Copy)]
struct SeriesL {
    b: Var,
    slots: [Slot; 5],
    state: usize,
}

impl SeriesL {
    fn setup(s: &mut Setup<'_>, name: &str, p: Var, n: Var) -> Self {
        let b = s.internal_branch(name, "esl");
        let slots = [s.slot(p, b), s.slot(n, b), s.slot(b, p), s.slot(b, n), s.slot(b, b)];
        Self {
            b,
            slots,
            state: s.states(2),
        }
    }
    fn load(&self, cx: &mut LoadCtx<'_>, henries: f64) {
        let [pb, nb, bp, bn, bb] = self.slots;
        cx.add(pb, 1.0);
        cx.add(nb, -1.0);
        cx.add(bp, 1.0);
        cx.add(bn, -1.0);
        let i = cx.v(self.b);
        cx.states.now[self.state] = henries * i;
        match cx.mode {
            Mode::Dc => {}
            Mode::TranInit => cx.states.now[self.state + 1] = 0.0,
            Mode::Tran => {
                let (req, v) = cx.integrate(self.state, henries);
                cx.add(bb, -req);
                cx.rhs(self.b, v - req * i);
            }
        }
    }
}

impl Capacitor {
    /// Capacitor of `farads` between `p` and `n`.
    pub fn new(name: &str, p: Var, n: Var, farads: f64, ic: Option<f64>) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            farads,
            ic,
            tolerance: None,
            esr: 0.0,
            esl: 0.0,
            cp: p,
            s: [Slot::NONE; 4],
            s_esr: [Slot::NONE; 4],
            esl_branch: None,
            state: 0,
        }
    }

    /// Add parasitic series resistance and inductance.
    pub fn with_parasitics(mut self, esr: f64, esl: f64) -> Self {
        self.esr = esr;
        self.esl = esl;
        self
    }

    /// Record a relative tolerance.
    pub fn with_tolerance(mut self, tol: Option<f64>) -> Self {
        self.tolerance = tol;
        self
    }
}

impl Device for Capacitor {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        // p -- ESR -- m -- ESL -- cp -- C -- n
        let mut node = self.p;
        if self.esr > 0.0 {
            let m = s.internal_node(&self.name, "esr");
            self.s_esr = s.slots2(node, m);
            node = m;
        }
        if self.esl > 0.0 {
            let m = s.internal_node(&self.name, "esl");
            self.esl_branch = Some(SeriesL::setup(s, &self.name, node, m));
            node = m;
        }
        self.cp = node;
        self.s = s.slots2(self.cp, self.n);
        self.state = s.states(2);
    }
    fn initial_condition(&self, x: &mut [f64]) {
        if let Some(ic) = self.ic {
            if self.n == GROUND {
                x[self.cp] = ic;
            } else if self.cp == GROUND {
                x[self.n] = -ic;
            } else {
                x[self.cp] = x[self.n] + ic;
            }
        }
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        if self.esr > 0.0 {
            cx.conductance(&self.s_esr, 1.0 / self.esr);
        }
        if let Some(l) = self.esl_branch {
            l.load(cx, self.esl);
        }
        let v = cx.v(self.cp) - cx.v(self.n);
        match cx.mode {
            Mode::Dc => cx.states.now[self.state] = self.farads * v,
            Mode::TranInit => {
                let v0 = self.ic.unwrap_or(v);
                cx.states.now[self.state] = self.farads * v0;
                cx.states.now[self.state + 1] = 0.0;
            }
            Mode::Tran => {
                cx.states.now[self.state] = self.farads * v;
                let (geq, i) = cx.integrate(self.state, self.farads);
                cx.conductance(&self.s, geq);
                cx.current(self.cp, self.n, i - geq * v);
            }
        }
    }
}

/// Linear inductor; its current is an MNA unknown `i(name)`.
#[derive(Debug, Clone)]
pub struct Inductor {
    name: String,
    p: Var,
    n: Var,
    /// Inductance in henries.
    pub henries: f64,
    /// Initial current for `uic` runs.
    pub ic: Option<f64>,
    /// Relative tolerance, for Monte Carlo analyses.
    pub tolerance: Option<f64>,
    /// Series (winding) resistance (Ω), 0 for none.
    pub rser: f64,
    /// Node at the far end of the winding resistance.
    pi: Var,
    s_r: [Slot; 4],
    b: Var,
    pb: Slot,
    nb: Slot,
    bp: Slot,
    bn: Slot,
    bb: Slot,
    state: usize,
}

impl Inductor {
    /// Inductor of `henries` from `p` to `n`.
    pub fn new(name: &str, p: Var, n: Var, henries: f64, ic: Option<f64>) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            henries,
            ic,
            tolerance: None,
            rser: 0.0,
            pi: p,
            s_r: [Slot::NONE; 4],
            b: 0,
            pb: Slot::NONE,
            nb: Slot::NONE,
            bp: Slot::NONE,
            bn: Slot::NONE,
            bb: Slot::NONE,
            state: 0,
        }
    }

    /// Add a series winding resistance.
    pub fn with_series_resistance(mut self, rser: f64) -> Self {
        self.rser = rser;
        self
    }

    /// Record a relative tolerance.
    pub fn with_tolerance(mut self, tol: Option<f64>) -> Self {
        self.tolerance = tol;
        self
    }
}

impl Device for Inductor {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        0
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        if self.rser > 0.0 {
            self.pi = s.internal_node(&self.name, "r");
            self.s_r = s.slots2(self.p, self.pi);
        }
        self.b = s.branch(&self.name);
        self.pb = s.slot(self.pi, self.b);
        self.nb = s.slot(self.n, self.b);
        self.bp = s.slot(self.b, self.pi);
        self.bn = s.slot(self.b, self.n);
        self.bb = s.slot(self.b, self.b);
        self.state = s.states(2);
    }
    fn branch(&self) -> Option<Var> {
        Some(self.b)
    }
    fn initial_condition(&self, x: &mut [f64]) {
        if let Some(i) = self.ic {
            x[self.b] = i;
        }
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        if self.rser > 0.0 {
            cx.conductance(&self.s_r, 1.0 / self.rser);
        }
        cx.add(self.pb, 1.0);
        cx.add(self.nb, -1.0);
        cx.add(self.bp, 1.0);
        cx.add(self.bn, -1.0);
        let i = cx.v(self.b);
        match cx.mode {
            Mode::Dc => cx.states.now[self.state] = self.henries * i,
            Mode::TranInit => {
                cx.states.now[self.state] = self.henries * i;
                cx.states.now[self.state + 1] = 0.0;
            }
            Mode::Tran => {
                cx.states.now[self.state] = self.henries * i;
                let (req, v) = cx.integrate(self.state, self.henries);
                // v_p - v_n - req * i = v - req * i
                cx.add(self.bb, -req);
                cx.rhs(self.b, v - req * i);
            }
        }
    }
}

/// Mutual inductance `M = k·sqrt(L1·L2)` between two inductors (SPICE `K`).
/// Adds `M·di2/dt` to inductor 1's branch equation and `M·di1/dt` to
/// inductor 2's, with the dot convention on each inductor's first node.
#[derive(Debug, Clone)]
pub struct MutualInductance {
    name: String,
    l1: String,
    l2: String,
    /// Mutual inductance (H).
    pub m: f64,
    b1: Var,
    b2: Var,
    s12: Slot,
    s21: Slot,
    state: usize,
}

impl MutualInductance {
    /// Coupling of inductors named `l1` and `l2` with mutual inductance `m`.
    pub fn new(name: &str, l1: &str, l2: &str, m: f64) -> Self {
        Self {
            name: name.into(),
            l1: l1.into(),
            l2: l2.into(),
            m,
            b1: 0,
            b2: 0,
            s12: Slot::NONE,
            s21: Slot::NONE,
            state: 0,
        }
    }
}

impl Device for MutualInductance {
    fn name(&self) -> &str {
        &self.name
    }
    fn setup_priority(&self) -> u8 {
        2
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        self.b1 = s.branch_of(&self.l1).unwrap_or(GROUND);
        self.b2 = s.branch_of(&self.l2).unwrap_or(GROUND);
        self.s12 = s.slot(self.b1, self.b2);
        self.s21 = s.slot(self.b2, self.b1);
        // States: M·i2 (for branch 1) and M·i1 (for branch 2).
        self.state = s.states(4);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let (i1, i2) = (cx.v(self.b1), cx.v(self.b2));
        let st = self.state;
        cx.states.now[st] = self.m * i2;
        cx.states.now[st + 2] = self.m * i1;
        match cx.mode {
            Mode::Dc => {}
            Mode::TranInit => {
                cx.states.now[st + 1] = 0.0;
                cx.states.now[st + 3] = 0.0;
            }
            Mode::Tran => {
                let (g, v1) = cx.integrate(st, self.m);
                cx.add(self.s12, -g);
                cx.rhs(self.b1, v1 - g * i2);
                let (g, v2) = cx.integrate(st + 2, self.m);
                cx.add(self.s21, -g);
                cx.rhs(self.b2, v2 - g * i1);
            }
        }
    }
}
