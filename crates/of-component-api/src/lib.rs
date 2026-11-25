//! Component API for the base engine (RFC 0001 / design §3b).
//!
//! Built-in models and (later) plugins implement [`Component`]. The engine
//! owns nets and the analog solver; components elaborate into
//! [`of_analog::Device`]s and report [`Component::terminal_currents`].

use of_analog::{AnalogEngine, Circuit, Device, Var};
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

/// A typed parameter value from a circuit file or the host (E4b, RFC 0001
/// addendum). Solver parameters stay `f64`; the other kinds are for
/// component options such as `inputs = ["GPIO9"]`.
#[derive(Debug, Clone, PartialEq)]
pub enum ParamValue {
    /// A number (SI units).
    Number(f64),
    /// A flag.
    Bool(bool),
    /// A string.
    Text(String),
    /// A list of strings.
    List(Vec<String>),
}

impl ParamValue {
    /// The value as a number (`true` is 1, `false` is 0).
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Self::Number(v) => Some(*v),
            Self::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            _ => None,
        }
    }

    /// The value as a flag (a number is true when it is not 0).
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool(b) => Some(*b),
            Self::Number(v) => Some(*v != 0.0),
            _ => None,
        }
    }

    /// The value as text.
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Self::Text(s) => Some(s),
            _ => None,
        }
    }

    /// The value as a list of strings (a single string is a list of one).
    pub fn as_str_list(&self) -> Option<Vec<&str>> {
        match self {
            Self::Text(s) => Some(vec![s.as_str()]),
            Self::List(items) => Some(items.iter().map(String::as_str).collect()),
            _ => None,
        }
    }
}

impl From<f64> for ParamValue {
    fn from(v: f64) -> Self {
        Self::Number(v)
    }
}

impl From<bool> for ParamValue {
    fn from(v: bool) -> Self {
        Self::Bool(v)
    }
}

impl From<&str> for ParamValue {
    fn from(v: &str) -> Self {
        Self::Text(v.to_owned())
    }
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
    /// Handle for a device already in the circuit, by instance name (for
    /// devices added through [`Self::circuit_mut`], such as bridge
    /// placements). `None` if there is no such device.
    fn device_ref(&self, _name: &str) -> Option<DeviceRef> {
        None
    }
}

/// What a component may do after elaboration, once the analog engine exists
/// and before the kernel starts (E4b): register watches and sample points,
/// read the operating point, and (for active parts) learn its engine ids.
pub trait AttachCtx {
    /// The analog engine (operating point already solved).
    fn analog(&mut self) -> &mut AnalogEngine;
    /// Kernel id of the analog engine.
    fn analog_id(&self) -> u32;
    /// Kernel id the engine from [`Component::take_engine`] will get, if this
    /// component reported [`Component::has_engine`].
    fn own_engine_id(&self) -> Option<u32>;
    /// Value of a named unknown in the operating point, for example `v(vdd)`.
    fn op_value(&self, name: &str) -> Option<f64>;
    /// Ask the session to start at `t` (a pre-booted CPU whose clock is ahead).
    /// The session runs every engine to the latest requested origin before
    /// handing control to the caller.
    fn set_origin(&mut self, t: SimTime);
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

    /// [`Self::commit`] with the right to change devices (for example a bench
    /// PSU switching between constant voltage and constant current). Changes
    /// take effect on the next analog step. The default calls [`Self::commit`].
    fn commit_with(&mut self, view: &dyn SolutionView, t: SimTime, _cx: &mut dyn EventCtx) -> Health {
        self.commit(view, t)
    }

    /// Hook into the analog engine once it exists (watches, sample points).
    fn attach(&mut self, _cx: &mut dyn AttachCtx) -> Result<(), ElabError> {
        Ok(())
    }

    /// True when [`Self::take_engine`] will return an engine.
    fn has_engine(&self) -> bool {
        false
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

#[cfg(test)]
mod tests {
    use super::*;

    struct Dummy {
        info: ComponentInfo,
        attached: bool,
        dev: DeviceRef,
    }

    impl Dummy {
        fn new() -> Self {
            Self {
                info: ComponentInfo::default(),
                attached: false,
                dev: DeviceRef(3),
            }
        }
    }

    impl Component for Dummy {
        fn info(&self) -> &ComponentInfo {
            &self.info
        }

        fn elaborate(&mut self, _cx: &mut dyn ElabCtx) -> Result<(), ElabError> {
            Ok(())
        }

        fn terminal_currents(&self, _view: &dyn SolutionView, out: &mut [f64]) {
            out.fill(0.0);
        }

        fn attach(&mut self, cx: &mut dyn AttachCtx) -> Result<(), ElabError> {
            self.attached = cx.own_engine_id().is_none() && cx.analog_id() == 7;
            Ok(())
        }

        fn commit_with(&mut self, view: &dyn SolutionView, _t: SimTime, cx: &mut dyn EventCtx) -> Health {
            let v = view.value("v(a)").unwrap_or(0.0);
            cx.set(self.dev, "r", v * 2.0).expect("set");
            Health::Ok
        }
    }

    struct FakeAttach(AnalogEngine);

    impl AttachCtx for FakeAttach {
        fn analog(&mut self) -> &mut AnalogEngine {
            &mut self.0
        }
        fn analog_id(&self) -> u32 {
            7
        }
        fn own_engine_id(&self) -> Option<u32> {
            None
        }
        fn op_value(&self, _name: &str) -> Option<f64> {
            None
        }
        fn set_origin(&mut self, _t: SimTime) {}
    }

    #[derive(Default)]
    struct Recorder(Vec<(DeviceRef, String, f64)>);

    impl EventCtx for Recorder {
        fn now(&self) -> SimTime {
            SimTime::ZERO
        }
        fn set(&mut self, dev: DeviceRef, param: &str, value: f64) -> Result<(), ElabError> {
            self.0.push((dev, param.to_owned(), value));
            Ok(())
        }
        fn schedule(&mut self, _at: SimTime, _code: u32) {}
    }

    struct Fixed;

    impl SolutionView for Fixed {
        fn value(&self, name: &str) -> Option<f64> {
            (name == "v(a)").then_some(1.5)
        }
    }

    fn tiny_engine() -> AnalogEngine {
        let mut c = Circuit::new();
        let a = c.node("a");
        c.add(of_analog::devices::Resistor::new("r1", a, 0, 1e3)).unwrap();
        AnalogEngine::new(
            "analog",
            c,
            of_analog::Options::default(),
            of_analog::engine::EngineConfig::default(),
        )
        .unwrap()
    }

    #[test]
    fn attach_gets_the_engine_ids() {
        let mut d = Dummy::new();
        let mut cx = FakeAttach(tiny_engine());
        d.attach(&mut cx).unwrap();
        assert!(d.attached);
        assert!(!d.has_engine());
    }

    #[test]
    fn commit_with_may_change_devices_and_defaults_to_commit() {
        let mut d = Dummy::new();
        let mut rec = Recorder::default();
        assert_eq!(d.commit_with(&Fixed, SimTime::ZERO, &mut rec), Health::Ok);
        assert_eq!(rec.0, [(DeviceRef(3), "r".to_owned(), 3.0)]);
    }

    #[test]
    fn param_values_convert() {
        assert_eq!(ParamValue::from(2.5).as_f64(), Some(2.5));
        assert_eq!(ParamValue::from(true).as_f64(), Some(1.0));
        assert_eq!(ParamValue::Number(0.0).as_bool(), Some(false));
        assert_eq!(ParamValue::from("GPIO9").as_str_list(), Some(vec!["GPIO9"]));
        assert_eq!(
            ParamValue::List(vec!["GPIO9".into(), "GPIO2".into()]).as_str_list(),
            Some(vec!["GPIO9", "GPIO2"])
        );
        assert_eq!(ParamValue::from("x").as_f64(), None);
    }
}
