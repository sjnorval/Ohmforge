//! `Session`: a built circuit running in the co-simulation kernel (E4b,
//! engine plan Task 12).
//!
//! [`BuiltEngine::into_session`] builds the analog engine, lets every
//! component [`Component::attach`] to it, registers the components' own
//! engines (CPUs) first and the analog engine last, and runs the kernel to the
//! latest origin a pre-booted chip asked for. The caller then drives time with
//! [`Session::run_until`], changes host parameters with [`Session::set_param`]
//! and decides when components commit ([`Session::commit`]; the desktop demos
//! commit at each scope sample).

use std::collections::BTreeMap;

use of_analog::{AnalogEngine, EngineConfig, Options};
use of_component_api::{AttachCtx, ComponentEvent, DeviceRef, ElabError, EventCtx, Health, SolutionView};
use of_core::{Engine, EngineId, Kernel, SimTime, Stop};

use crate::{BuiltEngine, EngineError, PlacedComponent};

/// Kernel and solver settings for a run.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SessionConfig {
    /// Kernel quantum (the longest any engine runs ahead).
    pub quantum: SimTime,
    /// Longest analog time step (s).
    pub max_step: f64,
}

impl Default for SessionConfig {
    fn default() -> Self {
        Self {
            quantum: SimTime::from_us(50),
            max_step: 5e-6,
        }
    }
}

/// A running circuit: kernel, analog engine and live components.
pub struct Session {
    kernel: Kernel,
    analog: EngineId,
    components: Vec<PlacedComponent>,
    /// Kernel id of each component's own engine, if it has one.
    engines: Vec<Option<EngineId>>,
    /// Device names in circuit order (`DeviceRef(i)` is entry `i`).
    devices: Vec<String>,
    /// Pending component timers: (time, sequence) to (component, code).
    timers: BTreeMap<(SimTime, u64), (usize, u32)>,
    seq: u64,
    origin: SimTime,
}

impl std::fmt::Debug for Session {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Session")
            .field("now", &self.kernel.now())
            .field("origin", &self.origin)
            .field("components", &self.components.len())
            .field("timers", &self.timers.len())
            .finish()
    }
}

struct Attach<'a> {
    analog: &'a mut AnalogEngine,
    analog_id: u32,
    own: Option<u32>,
    origin: &'a mut SimTime,
}

impl AttachCtx for Attach<'_> {
    fn analog(&mut self) -> &mut AnalogEngine {
        self.analog
    }

    fn analog_id(&self) -> u32 {
        self.analog_id
    }

    fn own_engine_id(&self) -> Option<u32> {
        self.own
    }

    fn op_value(&self, name: &str) -> Option<f64> {
        self.analog.value(name)
    }

    fn set_origin(&mut self, t: SimTime) {
        *self.origin = (*self.origin).max(t);
    }
}

/// [`EventCtx`] that queues device changes and timers; the session applies
/// them after the component returns.
struct Queue<'a> {
    now: SimTime,
    devices: &'a [String],
    sets: Vec<(usize, String, f64)>,
    timers: Vec<(SimTime, u32)>,
}

impl EventCtx for Queue<'_> {
    fn now(&self) -> SimTime {
        self.now
    }

    fn set(&mut self, dev: DeviceRef, param: &str, value: f64) -> Result<(), ElabError> {
        let i = dev.0 as usize;
        if i >= self.devices.len() {
            return Err(ElabError::msg(format!("no device {i}")));
        }
        self.sets.push((i, param.to_owned(), value));
        Ok(())
    }

    fn schedule(&mut self, at: SimTime, code: u32) {
        self.timers.push((at, code));
    }
}

struct View<'a>(&'a AnalogEngine);

impl SolutionView for View<'_> {
    fn value(&self, name: &str) -> Option<f64> {
        self.0.value(name)
    }
}

impl BuiltEngine {
    /// Build the analog engine, call `attach` on every component, take the
    /// active engines (registered first, invariant 6), and set up the kernel.
    pub fn into_session(mut self, cfg: SessionConfig) -> Result<Session, EngineError> {
        let circuit = std::mem::replace(&mut self.circuit, of_analog::Circuit::new());
        let mut analog = AnalogEngine::new(
            "analog",
            circuit,
            Options::default(),
            EngineConfig {
                max_step: cfg.max_step,
                record: false,
                ..EngineConfig::default()
            },
        )
        .map_err(|e| EngineError::Invariant(format!("operating point: {e}")))?;

        let active = self.components.iter().filter(|c| c.component.has_engine()).count();
        let analog_id = u32::try_from(active).map_err(|_| EngineError::Invariant("too many engines".into()))?;
        let mut origin = SimTime::ZERO;
        let mut next_id = 0u32;
        let mut own_ids = Vec::with_capacity(self.components.len());
        for pc in &mut self.components {
            let own = pc.component.has_engine().then(|| {
                next_id += 1;
                next_id - 1
            });
            own_ids.push(own);
            let mut cx = Attach {
                analog: &mut analog,
                analog_id,
                own,
                origin: &mut origin,
            };
            pc.component.attach(&mut cx)?;
        }

        let mut kernel = Kernel::new(cfg.quantum);
        let mut engines = Vec::with_capacity(self.components.len());
        for (pc, own) in self.components.iter_mut().zip(&own_ids) {
            let Some(want) = own else {
                engines.push(None);
                continue;
            };
            let engine = pc.component.take_engine().ok_or_else(|| {
                EngineError::Invariant(format!(
                    "{}: reported an engine but take_engine returned none",
                    pc.component.info().refdes
                ))
            })?;
            let id = kernel.add_engine(engine);
            debug_assert_eq!(id.0, *want);
            engines.push(Some(id));
        }
        let analog = kernel.add_engine(Box::new(analog));
        debug_assert_eq!(analog.0, analog_id);

        let mut session = Session {
            kernel,
            analog,
            components: self.components,
            engines,
            devices: self.device_names,
            timers: BTreeMap::new(),
            seq: 0,
            origin,
        };
        if origin > SimTime::ZERO {
            // Catch the analog solver up to the pre-booted CPU (as PwmSim did).
            match session.kernel.run_until(origin).map_err(kernel_err)? {
                Stop::Reached(_) => {}
                other => {
                    return Err(EngineError::Invariant(format!(
                        "an engine stopped while connecting the circuit ({other:?})"
                    )));
                }
            }
        }
        Ok(session)
    }
}

fn kernel_err(e: impl std::fmt::Display) -> EngineError {
    EngineError::Invariant(format!("kernel: {e}"))
}

impl Session {
    /// Time since [`Self::origin`].
    pub fn now(&self) -> SimTime {
        SimTime(self.kernel.now().0.saturating_sub(self.origin.0))
    }

    /// Kernel time when the circuit was connected (non-zero for pre-booted chips).
    pub fn origin(&self) -> SimTime {
        self.origin
    }

    /// Absolute kernel time.
    pub fn kernel_now(&self) -> SimTime {
        self.kernel.now()
    }

    /// Run to `t` (relative to [`Self::origin`]), firing component timers on
    /// the way. A host event, breakpoint or engine stop ends the call early;
    /// times inside the returned [`Stop`] are absolute kernel times.
    pub fn run_until(&mut self, t: SimTime) -> Result<Stop, EngineError> {
        let until = self.origin.saturating_add(t);
        loop {
            self.fire_due_timers()?;
            let now = self.kernel.now();
            if now >= until {
                return Ok(Stop::Reached(now));
            }
            let next = self
                .timers
                .keys()
                .next()
                .map(|&(at, _)| at)
                .filter(|&at| at < until)
                .unwrap_or(until);
            match self.kernel.run_until(next).map_err(kernel_err)? {
                Stop::Reached(_) => {}
                other => return Ok(other),
            }
        }
    }

    fn fire_due_timers(&mut self) -> Result<(), EngineError> {
        let now = self.kernel.now();
        while let Some((&key, _)) = self.timers.iter().next() {
            if key.0 > now {
                break;
            }
            let (comp, code) = self.timers.remove(&key).expect("timer");
            self.deliver(comp, &ComponentEvent::Timer { code })?;
        }
        Ok(())
    }

    fn index(&self, refdes: &str) -> Result<usize, EngineError> {
        self.components
            .iter()
            .position(|c| c.component.info().refdes.eq_ignore_ascii_case(refdes))
            .ok_or_else(|| EngineError::Invariant(format!("no component '{refdes}'")))
    }

    fn analog_mut(&mut self) -> &mut AnalogEngine {
        self.kernel
            .engine_as::<AnalogEngine>(self.analog)
            .expect("analog engine is registered")
    }

    /// Apply queued device changes and timers from component `comp`.
    fn apply(
        &mut self,
        comp: usize,
        sets: Vec<(usize, String, f64)>,
        timers: Vec<(SimTime, u32)>,
    ) -> Result<(), EngineError> {
        for (dev, param, value) in sets {
            let name = self.devices[dev].clone();
            self.analog_mut()
                .set(&name, &param, value)
                .map_err(|e| EngineError::Invariant(format!("set {name}.{param}: {e}")))?;
        }
        for (at, code) in timers {
            self.seq += 1;
            self.timers.insert((at, self.seq), (comp, code));
        }
        Ok(())
    }

    fn deliver(&mut self, comp: usize, ev: &ComponentEvent) -> Result<(), EngineError> {
        let mut q = Queue {
            now: self.kernel.now(),
            devices: &self.devices,
            sets: Vec::new(),
            timers: Vec::new(),
        };
        self.components[comp].component.on_event(ev, &mut q);
        let Queue { sets, timers, .. } = q;
        self.apply(comp, sets, timers)
    }

    /// Call `commit_with` on every component, in placement order. Device
    /// changes they ask for take effect on the next analog step.
    pub fn commit(&mut self) -> Result<Vec<(String, Health)>, EngineError> {
        let now = self.kernel.now();
        let mut out = Vec::with_capacity(self.components.len());
        for i in 0..self.components.len() {
            let analog = self
                .kernel
                .engine_as::<AnalogEngine>(self.analog)
                .expect("analog engine is registered");
            let view = View(analog);
            let mut q = Queue {
                now,
                devices: &self.devices,
                sets: Vec::new(),
                timers: Vec::new(),
            };
            let pc = &mut self.components[i];
            let health = pc.component.commit_with(&view, now, &mut q);
            out.push((pc.component.info().refdes.clone(), health));
            let Queue { sets, timers, .. } = q;
            self.apply(i, sets, timers)?;
        }
        Ok(out)
    }

    /// Host parameter change on component `refdes` (pot `position`, switch
    /// `closed`, PSU `volts` ...). Device changes apply before the next step.
    pub fn set_param(&mut self, refdes: &str, name: &str, value: f64) -> Result<(), EngineError> {
        let i = self.index(refdes)?;
        self.deliver(
            i,
            &ComponentEvent::HostParam {
                name: name.to_owned(),
                value,
            },
        )
    }

    /// Latest value of `v(net)` or `i(device)`.
    pub fn value(&mut self, signal: &str) -> Option<f64> {
        self.analog_mut().value(signal)
    }

    /// Named readings of one component.
    pub fn readings(&mut self, refdes: &str) -> Vec<(String, f64)> {
        let Ok(i) = self.index(refdes) else {
            return Vec::new();
        };
        let analog = self
            .kernel
            .engine_as::<AnalogEngine>(self.analog)
            .expect("analog engine is registered");
        let mut out = Vec::new();
        self.components[i].component.readings(&View(analog), &mut out);
        out
    }

    /// Current into each pin of component `refdes` (A, pin order of its
    /// `info()`), from the latest solution; `None` for an unknown ref. The
    /// desktop meters read two-terminal branches through this until E5.
    pub fn terminal_currents(&mut self, refdes: &str) -> Option<Vec<f64>> {
        let i = self.index(refdes).ok()?;
        let analog = self
            .kernel
            .engine_as::<AnalogEngine>(self.analog)
            .expect("analog engine is registered");
        let c = &self.components[i].component;
        let mut out = vec![0.0; c.info().pins.len()];
        c.terminal_currents(&View(analog), &mut out);
        Some(out)
    }

    /// Kernel id of component `refdes`'s own engine.
    pub fn engine_id(&self, refdes: &str) -> Option<EngineId> {
        self.index(refdes).ok().and_then(|i| self.engines[i])
    }

    /// Downcast a component's engine, for example `Esp32c3Engine` for UART.
    pub fn engine_as<T: Engine>(&mut self, refdes: &str) -> Option<&mut T> {
        let id = self.engine_id(refdes)?;
        self.kernel.engine_as::<T>(id)
    }

    /// The analog engine (meters until E5).
    pub fn analog(&mut self) -> &mut AnalogEngine {
        self.analog_mut()
    }

    /// The kernel, for breakpoints and host events.
    pub fn kernel(&mut self) -> &mut Kernel {
        &mut self.kernel
    }

    /// Placed components, in placement order.
    pub fn components(&self) -> &[PlacedComponent] {
        &self.components
    }

    /// Pending component timers (absolute kernel time, component index, code).
    pub fn pending_timers(&self) -> impl Iterator<Item = (SimTime, usize, u32)> + '_ {
        self.timers.iter().map(|(&(t, _), &(c, code))| (t, c, code))
    }
}
