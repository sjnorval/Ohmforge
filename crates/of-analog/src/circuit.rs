//! Circuit container, the [`Device`] trait and the per-iteration load
//! context that devices stamp into.

use std::collections::BTreeMap;

use crate::integrate::{Integrator, States};
use crate::matrix::{Pattern, Slot, SparseSystem};

/// Variable (unknown) number. 0 is ground; 1..=n are matrix unknowns.
pub type Var = usize;

/// The ground variable.
pub const GROUND: Var = 0;

/// What an unknown represents.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VarKind {
    /// A node voltage.
    Voltage,
    /// A branch current (voltage sources, inductors, controlled sources).
    Current,
}

/// Description of one unknown.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VarInfo {
    /// Output name: `v(node)` or `i(device)`.
    pub name: String,
    /// Voltage or current.
    pub kind: VarKind,
    /// Device-internal (series-resistance nodes); hidden from default output.
    pub internal: bool,
}

/// Analysis mode seen by devices.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// DC operating point or sweep: capacitors open, inductors shorted.
    Dc,
    /// Compute charges at the initial point of a transient (no stamping needed).
    TranInit,
    /// Transient time step with companion models.
    Tran,
}

/// Transient settings that some source defaults depend on.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TranSpec {
    /// Output step (default rise/fall time of PULSE sources).
    pub tstep: f64,
    /// Stop time (default period/width of PULSE, frequency of SIN).
    pub tstop: f64,
}

impl Default for TranSpec {
    fn default() -> Self {
        Self { tstep: 1.0, tstop: 1.0 }
    }
}

/// Device-facing view of setup: allocate unknowns, matrix entries and state.
pub struct Setup<'a> {
    vars: &'a mut Vec<VarInfo>,
    pattern: &'a mut Pattern,
    n_states: &'a mut usize,
    branches: &'a BTreeMap<String, Var>,
}

impl Setup<'_> {
    /// A device-internal node, e.g. behind a series resistance.
    pub fn internal_node(&mut self, device: &str, suffix: &str) -> Var {
        self.vars.push(VarInfo {
            name: format!("v({device}#{suffix})"),
            kind: VarKind::Voltage,
            internal: true,
        });
        self.vars.len() - 1
    }

    /// A branch-current unknown named `i(name)`.
    pub fn branch(&mut self, name: &str) -> Var {
        self.vars.push(VarInfo {
            name: format!("i({name})"),
            kind: VarKind::Current,
            internal: false,
        });
        self.vars.len() - 1
    }

    /// A device-internal branch current named `i(device#suffix)`, hidden
    /// from default output (e.g. a capacitor's ESL).
    pub fn internal_branch(&mut self, device: &str, suffix: &str) -> Var {
        self.vars.push(VarInfo {
            name: format!("i({device}#{suffix})"),
            kind: VarKind::Current,
            internal: true,
        });
        self.vars.len() - 1
    }

    /// Branch current of another device (for F/H sources); available once
    /// branch-owning devices are set up.
    pub fn branch_of(&self, device: &str) -> Option<Var> {
        self.branches.get(device).copied()
    }

    /// Matrix entry at (`r`, `c`).
    pub fn slot(&mut self, r: Var, c: Var) -> Slot {
        self.pattern.slot(r, c)
    }

    /// The four entries of a two-terminal conductance: (pp, pn, np, nn).
    pub fn slots2(&mut self, p: Var, n: Var) -> [Slot; 4] {
        [self.slot(p, p), self.slot(p, n), self.slot(n, p), self.slot(n, n)]
    }

    /// Reserve `k` state slots; returns the first index.
    pub fn states(&mut self, k: usize) -> usize {
        let base = *self.n_states;
        *self.n_states += k;
        base
    }
}

/// Everything a device sees while loading one Newton iteration.
pub struct LoadCtx<'a> {
    /// Current iterate in variable numbering (`x[0]` is ground, always 0).
    pub x: &'a [f64],
    mat: &'a mut SparseSystem,
    rhs: &'a mut [f64],
    /// Analysis mode.
    pub mode: Mode,
    /// First iteration of an operating point: initialise junction voltages.
    pub init_junctions: bool,
    /// Independent-source scale factor (source stepping), normally 1.
    pub src_factor: f64,
    /// Conductance added in parallel with every pn junction.
    pub gmin: f64,
    /// Simulation time in seconds (transient), 0 in DC.
    pub time: f64,
    /// Transient settings for source defaults.
    pub tran: TranSpec,
    /// Operating point at the start of a transient: sources use their
    /// waveform value at t = 0 rather than their DC value (as in SPICE).
    pub tran_op: bool,
    /// Integration coefficients for the current step.
    pub integ: &'a Integrator,
    /// Charge and current history.
    pub states: &'a mut States,
    /// Set by a device when it limited a junction voltage this iteration
    /// (the iteration cannot count as converged).
    pub limited: bool,
    /// Linearising about a converged operating point for `.ac`: devices
    /// must not limit voltages or advance internal state.
    pub small_signal: bool,
    /// Transient rescue: switches keep their state from the start of the
    /// step instead of re-deciding it each iteration.
    pub freeze_switches: bool,
}

impl LoadCtx<'_> {
    /// Voltage (or current) of variable `v` in the current iterate.
    #[inline]
    pub fn v(&self, v: Var) -> f64 {
        self.x[v]
    }

    /// Add to a matrix entry.
    #[inline]
    pub fn add(&mut self, s: Slot, val: f64) {
        self.mat.add(s, val);
    }

    /// Add to the right-hand side of row `v`.
    #[inline]
    pub fn rhs(&mut self, v: Var, val: f64) {
        if v != GROUND {
            self.rhs[v] += val;
        }
    }

    /// Stamp conductance `g` using slots from [`Setup::slots2`].
    #[inline]
    pub fn conductance(&mut self, s: &[Slot; 4], g: f64) {
        self.add(s[0], g);
        self.add(s[1], -g);
        self.add(s[2], -g);
        self.add(s[3], g);
    }

    /// Stamp a current `i` flowing from `p` to `n` through the device.
    #[inline]
    pub fn current(&mut self, p: Var, n: Var, i: f64) {
        self.rhs(p, -i);
        self.rhs(n, i);
    }

    /// Integrate the charge stored at `states[k]` (whose capacitance is
    /// `cap`): writes the current into `states[k + 1]` and returns
    /// `(geq, i)` where `geq = ag0 * cap` is the companion conductance.
    pub fn integrate(&mut self, k: usize, cap: f64) -> (f64, f64) {
        let i = self.integ.derivative(self.states, k);
        self.states.now[k + 1] = i;
        self.states.charges[k] = true;
        (self.integ.ag0 * cap, i)
    }
}

/// Settings for one assembly of the system.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct LoadParams {
    pub mode: Mode,
    pub init_junctions: bool,
    pub src_factor: f64,
    pub gmin: f64,
    pub diag_gmin: f64,
    pub time: f64,
    pub tran: TranSpec,
    pub tran_op: bool,
    pub small_signal: bool,
    /// Hold every switch at its state at the start of the step (the
    /// transient rescue, when switch flips keep Newton from settling).
    pub freeze_switches: bool,
}

/// A circuit element that stamps into the MNA system.
pub trait Device: std::fmt::Debug + Send {
    /// Instance name (lower-case, as in the netlist).
    fn name(&self) -> &str;

    /// Devices that own a branch current set up first (priority 0) so that
    /// current-controlled sources can find them.
    fn setup_priority(&self) -> u8 {
        1
    }

    /// Allocate internal nodes, branches, matrix slots and state.
    fn setup(&mut self, s: &mut Setup<'_>);

    /// The branch-current unknown this device owns, if any.
    fn branch(&self) -> Option<Var> {
        None
    }

    /// Stamp the linearised device for the current iterate.
    fn load(&mut self, cx: &mut LoadCtx<'_>);

    /// Whether the device is nonlinear (affects only diagnostics).
    fn is_nonlinear(&self) -> bool {
        false
    }

    /// True when this device's contribution is ramping or oscillating at `t`.
    fn is_changing(&self, _t: f64, _tran: &TranSpec) -> bool {
        false
    }

    /// First breakpoint strictly after `t` (source corners), if any.
    fn next_breakpoint(&self, _t: f64, _tran: &TranSpec) -> Option<f64> {
        None
    }

    /// A state change inside the step from `x` to `x_new` (a switch that
    /// flipped): the control's signed distance from the threshold it
    /// crossed at the start and the end of the step, so the transient can
    /// shorten the step to the crossing. `states` holds the step's new
    /// (`now`) and last accepted (`prev`) device states.
    fn state_crossing(&self, _x: &[f64], _x_new: &[f64], _states: &crate::integrate::States) -> Option<(f64, f64)> {
        None
    }

    /// Override the DC value of an independent source (DC sweeps).
    /// Returns `false` for devices that are not independent sources.
    fn set_dc(&mut self, _value: f64) -> bool {
        false
    }

    /// Apply `uic` initial conditions to the starting solution.
    fn initial_condition(&self, _x: &mut [f64]) {}

    /// Add this device's small-signal (`.ac`) excitation to the complex
    /// right-hand side (`re`, `im`, indexed by variable; row 0 is ground).
    fn ac_stimulus(&self, _re: &mut [f64], _im: &mut [f64]) {}

    /// Derived outputs at solution `x`, e.g. an LED's optical power, as
    /// `(name, value)` pairs such as `("popt(d1)", 1e-3)`.
    fn probes(&self, _x: &[f64], _out: &mut Vec<(String, f64)>) {}

    /// Change a run-time parameter between time slices (the scheduler
    /// engine; for example a pin driver's `v` and `r`, or a source's `dc`).
    /// Returns `false` if the device has no such parameter. The matrix
    /// pattern must not change.
    fn set_param(&mut self, _name: &str, _value: f64) -> bool {
        false
    }

    /// Current value of a run-time parameter, if this device has it.
    fn get_param(&self, _name: &str) -> Option<f64> {
        None
    }
}

/// Build errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum CircuitError {
    /// Two devices share a name.
    #[error("duplicate device name '{0}'")]
    Duplicate(String),
    /// A current-controlled source refers to a missing voltage source.
    #[error("'{0}' is controlled by unknown voltage source '{1}'")]
    UnknownControl(String, String),
}

/// A circuit ready for analysis.
#[derive(Debug)]
pub struct Circuit {
    pub(crate) vars: Vec<VarInfo>,
    node_names: BTreeMap<String, Var>,
    pub(crate) devices: Vec<Box<dyn Device>>,
    pub(crate) n_states: usize,
    pattern: Pattern,
    finalized: bool,
    pub(crate) system: Option<SparseSystem>,
}

impl Default for Circuit {
    fn default() -> Self {
        Self::new()
    }
}

impl Circuit {
    /// An empty circuit containing only ground (`0`, alias `gnd`).
    pub fn new() -> Self {
        let mut node_names = BTreeMap::new();
        node_names.insert("0".to_string(), GROUND);
        node_names.insert("gnd".to_string(), GROUND);
        Self {
            vars: vec![VarInfo {
                name: "v(0)".into(),
                kind: VarKind::Voltage,
                internal: true,
            }],
            node_names,
            devices: Vec::new(),
            n_states: 0,
            pattern: Pattern::default(),
            finalized: false,
            system: None,
        }
    }

    /// The variable for node `name`, creating it on first use.
    pub fn node(&mut self, name: &str) -> Var {
        let key = name.to_ascii_lowercase();
        if let Some(&v) = self.node_names.get(&key) {
            return v;
        }
        assert!(!self.finalized, "nodes must be created before analysis");
        self.vars.push(VarInfo {
            name: format!("v({key})"),
            kind: VarKind::Voltage,
            internal: false,
        });
        let v = self.vars.len() - 1;
        self.node_names.insert(key, v);
        v
    }

    /// Look up an existing node.
    pub fn find_node(&self, name: &str) -> Option<Var> {
        self.node_names.get(&name.to_ascii_lowercase()).copied()
    }

    /// Add a device. Names must be unique.
    pub fn add(&mut self, device: impl Device + 'static) -> Result<(), CircuitError> {
        self.add_boxed(Box::new(device))
    }

    /// Add a boxed device.
    pub fn add_boxed(&mut self, device: Box<dyn Device>) -> Result<(), CircuitError> {
        assert!(!self.finalized, "devices must be added before analysis");
        if self.devices.iter().any(|d| d.name() == device.name()) {
            return Err(CircuitError::Duplicate(device.name().to_string()));
        }
        self.devices.push(device);
        Ok(())
    }

    /// Devices in the circuit.
    pub fn devices(&self) -> impl Iterator<Item = &dyn Device> {
        self.devices.iter().map(|d| d.as_ref())
    }

    /// A device by instance name.
    pub fn device(&self, name: &str) -> Option<&dyn Device> {
        self.devices.iter().find(|d| d.name() == name).map(|d| d.as_ref())
    }

    /// Mutable access to a device by name.
    pub fn device_mut(&mut self, name: &str) -> Option<&mut Box<dyn Device>> {
        self.devices.iter_mut().find(|d| d.name() == name)
    }

    /// Number of unknowns (after [`Circuit::finalize`]).
    pub fn unknowns(&self) -> usize {
        self.vars.len() - 1
    }

    /// Description of every variable (index 0 is ground).
    pub fn vars(&self) -> &[VarInfo] {
        &self.vars
    }

    /// The variable named `v(node)` or `i(device)` (case-insensitive);
    /// a bare name is taken as a node. Branch currents and internal nodes
    /// exist only after [`Circuit::finalize`].
    pub fn var_named(&self, name: &str) -> Option<Var> {
        let key = name.to_ascii_lowercase();
        if !key.contains('(') {
            return self.find_node(&key);
        }
        self.vars.iter().position(|v| v.name == key)
    }

    /// Run device setup and build the sparse system. Called automatically by
    /// the analyses; idempotent.
    pub fn finalize(&mut self) -> Result<(), CircuitError> {
        if self.finalized {
            return Ok(());
        }
        let mut order: Vec<usize> = (0..self.devices.len()).collect();
        order.sort_by_key(|&i| self.devices[i].setup_priority());
        let mut branches = BTreeMap::new();
        for i in order {
            let dev = &mut self.devices[i];
            let mut s = Setup {
                vars: &mut self.vars,
                pattern: &mut self.pattern,
                n_states: &mut self.n_states,
                branches: &branches,
            };
            dev.setup(&mut s);
            if let Some(b) = dev.branch() {
                branches.insert(dev.name().to_string(), b);
            }
        }
        self.system = Some(SparseSystem::new(self.vars.len() - 1, &mut self.pattern));
        self.finalized = true;
        Ok(())
    }

    /// Whether every device is linear.
    pub fn is_linear(&self) -> bool {
        self.devices.iter().all(|d| !d.is_nonlinear())
    }

    /// Assemble the system for iterate `x`. Returns whether any device
    /// limited its junction voltages. `rhs` must have `unknowns() + 1` entries.
    pub(crate) fn load(
        &mut self,
        x: &[f64],
        rhs: &mut [f64],
        p: &LoadParams,
        integ: &Integrator,
        states: &mut States,
    ) -> bool {
        let system = self.system.as_mut().expect("finalize() before load");
        system.clear();
        rhs.iter_mut().for_each(|v| *v = 0.0);
        let mut cx = LoadCtx {
            x,
            mat: system,
            rhs,
            mode: p.mode,
            init_junctions: p.init_junctions,
            src_factor: p.src_factor,
            gmin: p.gmin,
            time: p.time,
            tran: p.tran,
            tran_op: p.tran_op,
            integ,
            states,
            limited: false,
            small_signal: p.small_signal,
            freeze_switches: p.freeze_switches,
        };
        for d in self.devices.iter_mut() {
            d.load(&mut cx);
        }
        let limited = cx.limited;
        let diag_gmin = p.diag_gmin;
        if diag_gmin > 0.0 {
            for (v, info) in self.vars.iter().enumerate().skip(1) {
                if info.kind == VarKind::Voltage {
                    let s = self.pattern.slot(v, v);
                    system.add(s, diag_gmin);
                }
            }
        }
        limited
    }
}
