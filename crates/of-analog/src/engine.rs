//! The analog solver as an [`of_core::Engine`].
//!
//! [`AnalogEngine`] runs a transient analysis slice by slice under the
//! `of-core` [`Kernel`](of_core::Kernel): `advance_to(t)` integrates up to
//! exactly `t` (picoseconds, converted to seconds), keeping the integration
//! history across slice ends, so a run cut into slices matches one long
//! `.tran` run within the truncation-error tolerance.
//!
//! * **Stimulus:** events bound with [`AnalogEngine::on_code`] (or direct
//!   [`AnalogEngine::set`] calls) change device parameters such as a
//!   [`PinDriver`](crate::devices::PinDriver)'s `v`/`r` or a source's `dc`.
//!   A change is a discontinuity: the next step restarts with backward Euler
//!   and a small step, as after a source corner.
//! * **Threshold watches:** [`AnalogEngine::watch`] reports a node crossing
//!   a level (for example a GPIO input threshold). The step that crosses is
//!   redone until it ends within `resolution` after the crossing; the engine
//!   then stops early there and posts the bound event, so the kernel shrinks
//!   the slice and the receiver sees the crossing time.
//! * **Sampling:** [`AnalogEngine::sample_on`] answers a request event
//!   with the present value of a signal, carried in the reply's code (a
//!   [`ValueCode`]) and sent at the same time; for example an ADC reading
//!   a node voltage.
//! * **Breakpoints:** device breakpoints (source corners) land exactly, as
//!   in `.tran`; kernel breakpoints and events end slices, which the solver
//!   also lands on exactly.

use std::collections::BTreeMap;

use of_core::{Engine, EngineError, Event, SimTime, SyncCtx, ValueCode};

use crate::analysis::tran::{Crossing, StepEnd, Stepper};
use crate::analysis::{AnalysisError, Options, TranRequest};
use crate::circuit::Circuit;
use crate::results::{TranStats, Waveforms};

/// Settings of an [`AnalogEngine`].
#[derive(Debug, Clone, PartialEq)]
pub struct EngineConfig {
    /// Largest internal time step (s).
    pub max_step: f64,
    /// Crossing-time resolution of threshold watches (s).
    pub resolution: f64,
    /// Time scale for source defaults (PULSE period, SIN frequency) as the
    /// `tstop` of a `.tran` would be (s).
    pub horizon: f64,
    /// Start from initial conditions instead of an operating point.
    pub uic: bool,
    /// Node initial conditions for `uic`.
    pub ic: BTreeMap<String, f64>,
    /// Keep every accepted point (for plots and tests).
    pub record: bool,
}

impl Default for EngineConfig {
    fn default() -> Self {
        Self {
            max_step: 10e-6,
            resolution: 100e-12,
            horizon: 1.0,
            uic: false,
            ic: BTreeMap::new(),
            record: false,
        }
    }
}

/// Which direction of a threshold crossing to report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    /// From below to at-or-above the level.
    Rising,
    /// From above to at-or-below the level.
    Falling,
    /// Either direction.
    Both,
}

/// A parameter change applied when an event code arrives.
#[derive(Debug, Clone, PartialEq)]
pub struct SetParam {
    /// Device name (lower-case, as in the circuit).
    pub device: String,
    /// Parameter, e.g. `v`, `r` or `dc`.
    pub param: String,
    /// New value.
    pub value: f64,
}

#[derive(Debug, Clone)]
struct Sample {
    signal: String,
    target: u32,
    reply: ValueCode,
}

#[derive(Debug, Clone)]
struct Watch {
    crossing: Crossing,
    target: u32,
    code: u32,
    enabled: bool,
}

/// Error from the engine's direct API.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum EngineApiError {
    /// No device of that name.
    #[error("no device named '{0}'")]
    UnknownDevice(String),
    /// The device does not have that run-time parameter.
    #[error("'{0}' has no run-time parameter '{1}'")]
    UnknownParam(String, String),
    /// No unknown of that name.
    #[error("no signal named '{0}'")]
    UnknownSignal(String),
}

/// A circuit under transient analysis, advanced by the `of-core` kernel.
#[derive(Debug)]
pub struct AnalogEngine {
    name: String,
    ckt: Circuit,
    opts: Options,
    st: Stepper,
    time: SimTime,
    resolution: f64,
    watches: Vec<Watch>,
    actions: BTreeMap<u32, Vec<SetParam>>,
    samples: BTreeMap<u32, Sample>,
    rec: Option<Waveforms>,
    changed: bool,
}

/// Index of a watch returned by [`AnalogEngine::watch`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchId(pub usize);

impl AnalogEngine {
    /// Compute the operating point (or apply `uic`) and get ready to run.
    pub fn new(name: &str, mut ckt: Circuit, opts: Options, cfg: EngineConfig) -> Result<Self, AnalysisError> {
        if !(cfg.max_step > 0.0 && cfg.resolution > 0.0 && cfg.horizon > 0.0) {
            return Err(AnalysisError::Invalid(
                "engine max_step, resolution and horizon must be positive".into(),
            ));
        }
        let req = TranRequest {
            tstep: cfg.max_step,
            tstop: cfg.horizon,
            tstart: 0.0,
            tmax: cfg.max_step,
            uic: cfg.uic,
            ic: cfg.ic.clone(),
        };
        let st = Stepper::start(&mut ckt, &opts, &req, f64::INFINITY)?;
        let rec = cfg.record.then(|| {
            let mut w = Waveforms::new(&ckt, "time".into());
            w.push(&ckt, 0.0, &st.x);
            w
        });
        Ok(Self {
            name: name.to_owned(),
            ckt,
            opts,
            st,
            time: SimTime::ZERO,
            resolution: cfg.resolution,
            watches: Vec::new(),
            actions: BTreeMap::new(),
            samples: BTreeMap::new(),
            rec,
            changed: false,
        })
    }

    /// The circuit (read-only; change parameters through [`Self::set`]).
    pub fn circuit(&self) -> &Circuit {
        &self.ckt
    }

    /// A node voltage `v(n)` (or bare node name) or branch current `i(x)`
    /// at the engine's current time; device probes such as `i(gpio)` of a
    /// pin driver or `popt(d1)` of an LED are also found.
    pub fn value(&self, name: &str) -> Option<f64> {
        if let Some(v) = self.ckt.var_named(name) {
            return Some(self.st.x[v]);
        }
        let key = name.to_ascii_lowercase();
        crate::results::probes(&self.ckt, &self.st.x)
            .into_iter()
            .find(|(n, _)| *n == key)
            .map(|(_, v)| v)
    }

    /// Set a device's run-time parameter now (a discontinuity).
    pub fn set(&mut self, device: &str, param: &str, value: f64) -> Result<(), EngineApiError> {
        let d = self
            .ckt
            .device_mut(device)
            .ok_or_else(|| EngineApiError::UnknownDevice(device.to_owned()))?;
        if !d.set_param(param, value) {
            return Err(EngineApiError::UnknownParam(device.to_owned(), param.to_owned()));
        }
        self.changed = true;
        Ok(())
    }

    /// Apply `changes` whenever an event with `code` arrives.
    pub fn on_code(&mut self, code: u32, changes: Vec<SetParam>) {
        self.actions.insert(code, changes);
    }

    /// Whenever an event with `code` arrives, send `target` the value of
    /// `signal` (as [`Self::value`] names it) at that moment, encoded with
    /// `reply`, at the same time. Parameter changes arriving at the same
    /// time are not yet solved for, so the value is the one just before
    /// them.
    pub fn sample_on(&mut self, code: u32, signal: &str, target: u32, reply: ValueCode) -> Result<(), EngineApiError> {
        self.value(signal)
            .ok_or_else(|| EngineApiError::UnknownSignal(signal.to_owned()))?;
        self.samples.insert(
            code,
            Sample {
                signal: signal.to_owned(),
                target,
                reply,
            },
        );
        Ok(())
    }

    /// Report crossings of `signal` through `level` as event `(target,
    /// code)` at the crossing time; the engine stops there.
    pub fn watch(
        &mut self,
        signal: &str,
        level: f64,
        edge: Edge,
        target: u32,
        code: u32,
    ) -> Result<WatchId, EngineApiError> {
        let var = self
            .ckt
            .var_named(signal)
            .ok_or_else(|| EngineApiError::UnknownSignal(signal.to_owned()))?;
        self.watches.push(Watch {
            crossing: Crossing {
                var,
                level,
                rising: edge != Edge::Falling,
                falling: edge != Edge::Rising,
            },
            target,
            code,
            enabled: true,
        });
        Ok(WatchId(self.watches.len() - 1))
    }

    /// Enable or disable a watch.
    pub fn enable_watch(&mut self, id: WatchId, on: bool) {
        if let Some(w) = self.watches.get_mut(id.0) {
            w.enabled = on;
        }
    }

    /// Recorded waveforms (with [`EngineConfig::record`]).
    pub fn waveforms(&self) -> Option<&Waveforms> {
        self.rec.as_ref()
    }

    /// Step statistics so far.
    pub fn stats(&self) -> TranStats {
        self.st.finish_stats(&self.ckt)
    }

    fn error(&self, e: impl std::fmt::Display) -> EngineError {
        EngineError {
            engine: self.name.clone(),
            at: self.time,
            message: e.to_string(),
        }
    }
}

impl Engine for AnalogEngine {
    fn name(&self) -> &str {
        &self.name
    }

    fn local_time(&self) -> SimTime {
        self.time
    }

    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        if limit <= self.time {
            return Ok(self.time.max(limit));
        }
        if std::mem::take(&mut self.changed) {
            self.st.discontinuity(&self.ckt);
        }
        let target = limit.as_secs_f64();
        let active: Vec<(usize, Crossing)> = self
            .watches
            .iter()
            .enumerate()
            .filter(|(_, w)| w.enabled)
            .map(|(i, w)| (i, w.crossing))
            .collect();
        let crossings: Vec<Crossing> = active.iter().map(|(_, c)| *c).collect();
        while self.st.t < target - self.st.h_min {
            let end = self
                .st
                .step(&mut self.ckt, &self.opts, target, &crossings, self.resolution)
                .map_err(|e| self.error(e))?;
            if let Some(w) = self.rec.as_mut() {
                w.push(&self.ckt, self.st.t, &self.st.x);
            }
            if let StepEnd::Crossing(k) = end {
                let at = SimTime::from_secs_f64(self.st.t).clamp(SimTime(cx.now().0 + 1), limit);
                let w = &self.watches[active[k].0];
                cx.schedule(at, w.target, w.code);
                self.time = at;
                return Ok(at);
            }
        }
        self.time = limit;
        Ok(limit)
    }

    fn on_event(&mut self, ev: &Event, cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
        if let Some(s) = self.samples.get(&ev.code) {
            let v = self.value(&s.signal).unwrap_or(0.0);
            cx.schedule(ev.at, s.target, s.reply.encode(v));
        }
        let Some(changes) = self.actions.get(&ev.code).cloned() else {
            return Ok(());
        };
        for c in changes {
            self.set(&c.device, &c.param, c.value).map_err(|e| self.error(e))?;
        }
        Ok(())
    }
}
