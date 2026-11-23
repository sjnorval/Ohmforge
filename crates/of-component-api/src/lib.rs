//! Component API for the base engine (RFC 0001 / design §3b).
//!
//! Built-in models and (later) plugins implement [`Component`]. The engine
//! owns nets and the analog solver; components elaborate into
//! [`of_analog::Device`]s and report [`Component::terminal_currents`].

use of_analog::{Circuit, Device, Var};
use of_core::{Engine, SimTime};

/// Result of committing a step: whether the part is still within ratings.
#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    /// Within ratings.
    Ok,
    /// Near or above a rating (for example junction temperature).
    Warning(String),
    /// Absolute maximum exceeded; the real part would be damaged.
    Damaged(String),
}

/// Faults a user can inject into a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// Open circuit between two pins.
    Open,
    /// Short circuit between two pins.
    Short,
    /// Pin stuck at a logic level.
    StuckAt,
    /// Parameter drifted out of tolerance.
    Drift,
}

/// Kind of a component pin (from `part.toml` / design §3g).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PinKind {
    /// Passive or unspecified.
    #[default]
    Passive,
    /// Power supply pin.
    Power,
    /// Ground / return pin.
    Ground,
    /// Digital or analog input.
    Input,
    /// Digital or analog output.
    Output,
    /// Bidirectional.
    Bidir,
}

/// One pin of a placed component.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PinSpec {
    /// Pin name as in `part.toml`.
    pub name: String,
    /// Electrical role.
    pub kind: PinKind,
}

/// Static description of a placed component instance.
#[derive(Debug, Clone, Default)]
pub struct ComponentInfo {
    /// Library part id, for example `generic.led-red-0603`.
    pub part_id: String,
    /// Reference designator, for example `D1`.
    pub refdes: String,
    /// Pins in declaration order (terminal_currents order).
    pub pins: Vec<PinSpec>,
}

/// Handle for a device added during elaboration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DeviceRef(pub u32);

/// Elaboration failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ElabError {
    /// Human-readable reason.
    #[error("{0}")]
    Message(String),
}

impl ElabError {
    /// Build from a displayable error.
    pub fn msg(e: impl ToString) -> Self {
        Self::Message(e.to_string())
    }
}

/// Event delivered to a component (watches, host commands, samples).
#[derive(Debug, Clone, PartialEq)]
pub enum ComponentEvent {
    /// Host or UI changed a named parameter (for example pot position).
    HostParam {
        /// Parameter name.
        name: String,
        /// New value.
        value: f64,
    },
    /// A scheduled timer fired.
    Timer {
        /// Opaque code chosen when the timer was scheduled.
        code: u32,
    },
}

/// Read-only view of the latest solution (voltages and branch currents).
pub trait SolutionView {
    /// Value of a named unknown or probe, for example `v(vdd)` or `i(r1)`.
    fn value(&self, name: &str) -> Option<f64>;
}

/// What a component may do while it is being built into the engine.
pub trait ElabCtx {
    /// Solver node for a pin of this component (never node 0 unless that pin
    /// is on the ground net).
    fn pin_node(&self, pin: &str) -> Result<Var, ElabError>;
    /// Allocate an internal node named `<refdes>.<name>`.
    fn internal_node(&mut self, name: &str) -> Var;
    /// Add a solver device; returns a handle for later `set` calls.
    fn add_device(&mut self, dev: Box<dyn Device>) -> Result<DeviceRef, ElabError>;
    /// Unfinalised circuit (for bridge helpers such as `add_placement`).
    fn circuit_mut(&mut self) -> &mut Circuit;
    /// Resolved placement / part parameter, if present.
    fn param(&self, name: &str) -> Option<f64>;
    /// Per-component seed (bounce, later noise).
    fn seed(&self) -> u64;
    /// Reference designator of the component being elaborated.
    fn refdes(&self) -> &str;
}

/// What a component may do when it handles an event or a host command.
pub trait EventCtx {
    /// Current simulation time.
    fn now(&self) -> SimTime;
    /// Change a device parameter (same matrix pattern).
    fn set(&mut self, dev: DeviceRef, param: &str, value: f64) -> Result<(), ElabError>;
    /// Schedule a timer event for this component.
    fn schedule(&mut self, at: SimTime, code: u32);
}

/// Behaviour of a simulated part (built-in Rust model or, later, a WASM plugin).
pub trait Component: Send {
    /// Static description (pins and identities).
    fn info(&self) -> &ComponentInfo;

    /// Pins and parameters (legacy name; same as [`Self::info`]).
    fn describe(&self) -> &ComponentInfo {
        self.info()
    }

    /// Add solver devices, internal nodes, watches and sample points.
    fn elaborate(&mut self, cx: &mut dyn ElabCtx) -> Result<(), ElabError>;

    /// Watches, timers, host commands, sample replies.
    fn on_event(&mut self, _ev: &ComponentEvent, _cx: &mut dyn EventCtx) {}

    /// Accepted step: state, dissipation, temperature, rating checks.
    fn commit(&mut self, _view: &dyn SolutionView, _t: SimTime) -> Health {
        Health::Ok
    }

    /// Current into each pin (A), in `info().pins` order; must sum to zero.
    fn terminal_currents(&self, view: &dyn SolutionView, out: &mut [f64]);

    /// Named internal readings, for example `("mode", 1.0)`.
    fn readings(&self, _view: &dyn SolutionView, _out: &mut Vec<(String, f64)>) {}

    /// Faults the user can inject.
    fn faults(&self) -> &[FaultKind] {
        &[]
    }

    /// Active components hand the engine their clocked engine (CPU, timers).
    fn take_engine(&mut self) -> Option<Box<dyn Engine>> {
        None
    }
}
