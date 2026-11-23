//! The engine trait and the time-slice kernel.
//!
//! Every simulation engine (CPU cores, peripherals, the digital kernel and
//! the analog solver) implements [`Engine`]. A [`Kernel`] owns the
//! [`Scheduler`] and advances its engines together in time slices:
//!
//! 1. Events due at the current time are delivered (`on_event`); events
//!    addressed to [`HOST`] stop the run and are returned to the caller.
//! 2. The slice ends at the earliest of `now + quantum`, the next scheduled
//!    event, the next breakpoint, the stop time and every engine's own
//!    [`Engine::next_event`] (its next self-timed wake-up).
//! 3. Engines advance in registration order. An engine may stop early (a
//!    CPU touching an I/O register, an analog threshold crossing); the
//!    engines after it then advance only to that instant, so the slice
//!    shrinks. Engines before it may be ahead by at most one quantum: put
//!    the engine that must see exact times (the analog solver) after the
//!    engines that request sync points (the CPU).
//!
//! The kernel is single-threaded and deterministic: the same engines,
//! events and quantum always give the same run.

use std::any::Any;
use std::collections::BTreeSet;
use std::fmt;

use crate::{Event, Scheduler, SimTime};

/// Event target that returns the event to the caller of [`Kernel::run_until`].
pub const HOST: u32 = u32::MAX;

/// Index of an engine in a [`Kernel`] (also its event target number).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EngineId(pub u32);

/// An engine failed (for example the analog solver did not converge).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EngineError {
    /// Engine name.
    pub engine: String,
    /// Simulation time of the failure.
    pub at: SimTime,
    /// What went wrong.
    pub message: String,
}

impl fmt::Display for EngineError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} at {} ps: {}", self.engine, self.at.0, self.message)
    }
}

impl std::error::Error for EngineError {}

/// What an engine may do while it advances or handles an event.
pub struct SyncCtx<'a> {
    sched: &'a mut Scheduler,
    now: SimTime,
    id: EngineId,
    stop: bool,
    /// Every registered engine reported [`Engine::is_quiescent`] at the
    /// start of this slice (after due events). Used to skip exact-time
    /// GPIO input deferrals when the analog side will only hold.
    peers_quiescent: bool,
}

impl SyncCtx<'_> {
    /// Kernel time at the start of the current slice (or of the event).
    pub fn now(&self) -> SimTime {
        self.now
    }
    /// The id of the engine being called.
    pub fn id(&self) -> EngineId {
        self.id
    }
    /// Whether every engine was quiescent when this slice started.
    pub fn peers_quiescent(&self) -> bool {
        self.peers_quiescent
    }
    /// Schedule an event for engine `target` (or [`HOST`]). Times before
    /// the slice start are clamped to it.
    pub fn schedule(&mut self, at: SimTime, target: u32, code: u32) {
        self.sched.schedule(at.max(self.now), target, code);
    }
    /// Ask the kernel to stop once the current slice has finished
    /// (for example a part reporting damage).
    pub fn request_stop(&mut self) {
        self.stop = true;
    }
}

/// A simulation engine advanced by the [`Kernel`].
///
/// `Any` lets callers reach the concrete engine through
/// [`Kernel::engine_as`].
pub trait Engine: Any + Send {
    /// Engine name, for errors and statistics.
    fn name(&self) -> &str;

    /// The engine's own time (how far it has advanced).
    fn local_time(&self) -> SimTime;

    /// Advance to `limit`. Returns the time reached:
    ///
    /// * `limit` in the normal case (discrete engines may overshoot by less
    ///   than one indivisible step, such as a CPU cycle);
    /// * an earlier time if the engine stopped early at a sync point; it
    ///   must still be after [`SyncCtx::now`] (engines must make progress).
    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError>;

    /// The next time this engine needs to run at exactly (a timer expiry, a
    /// PWM edge); the kernel ends a slice there. `None` if it has none.
    fn next_event(&self) -> Option<SimTime> {
        None
    }

    /// Handle an event addressed to this engine.
    fn on_event(&mut self, _ev: &Event, _cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
        Ok(())
    }

    /// Whether this engine will not spontaneously change the circuit or
    /// post events if advanced from its current state (for example a
    /// settled DC analog network). Defaults to `false`; engines that are
    /// known to be idle (or that never wake the network) override it.
    fn is_quiescent(&self) -> bool {
        false
    }
}

/// Why [`Kernel::run_until`] returned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Stop {
    /// The stop time was reached.
    Reached(SimTime),
    /// A breakpoint was reached (events due at that time are not yet
    /// delivered; they are when the run resumes).
    Breakpoint(SimTime),
    /// An event addressed to [`HOST`].
    Host(Event),
    /// An engine asked to stop.
    Requested(EngineId),
}

/// Counters for diagnostics and tests.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct KernelStats {
    /// Time slices run.
    pub slices: u64,
    /// Events delivered (including host events).
    pub events: u64,
    /// Slices shortened because an engine stopped early.
    pub early_stops: u64,
    /// Largest lead of an engine over kernel time seen after a slice.
    pub max_skew: SimTime,
}

/// Runs engines in time slices under one scheduler.
pub struct Kernel {
    sched: Scheduler,
    engines: Vec<Box<dyn Engine>>,
    now: SimTime,
    quantum: SimTime,
    breakpoints: BTreeSet<SimTime>,
    stats: KernelStats,
}

impl fmt::Debug for Kernel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Kernel")
            .field("now", &self.now)
            .field("quantum", &self.quantum)
            .field(
                "engines",
                &self.engines.iter().map(|e| e.name().to_owned()).collect::<Vec<_>>(),
            )
            .field("pending", &self.sched.pending())
            .finish()
    }
}

impl Kernel {
    /// New kernel at time zero. `quantum` is the longest slice (at least
    /// 1 ps).
    pub fn new(quantum: SimTime) -> Self {
        Self {
            sched: Scheduler::new(),
            engines: Vec::new(),
            now: SimTime::ZERO,
            quantum: SimTime(quantum.0.max(1)),
            breakpoints: BTreeSet::new(),
            stats: KernelStats::default(),
        }
    }

    /// Add an engine; engines advance in the order they were added.
    pub fn add_engine(&mut self, engine: Box<dyn Engine>) -> EngineId {
        self.engines.push(engine);
        EngineId(self.engines.len() as u32 - 1)
    }

    /// Kernel time: every engine has reached at least this time.
    pub fn now(&self) -> SimTime {
        self.now
    }

    /// Longest time slice.
    pub fn quantum(&self) -> SimTime {
        self.quantum
    }

    /// Change the longest time slice (at least 1 ps).
    pub fn set_quantum(&mut self, q: SimTime) {
        self.quantum = SimTime(q.0.max(1));
    }

    /// Counters.
    pub fn stats(&self) -> KernelStats {
        self.stats
    }

    /// Schedule an event (stimulus from the host). Past times are clamped
    /// to now.
    pub fn schedule(&mut self, at: SimTime, target: u32, code: u32) {
        self.sched.schedule(at.max(self.now), target, code);
    }

    /// Stop the run when time reaches `t` (one-shot).
    pub fn add_breakpoint(&mut self, t: SimTime) {
        if t >= self.now {
            self.breakpoints.insert(t);
        }
    }

    /// Remove a breakpoint; returns whether it existed.
    pub fn remove_breakpoint(&mut self, t: SimTime) -> bool {
        self.breakpoints.remove(&t)
    }

    /// An engine by id.
    pub fn engine(&self, id: EngineId) -> Option<&dyn Engine> {
        self.engines.get(id.0 as usize).map(|e| e.as_ref())
    }

    /// The concrete engine `T` with this id.
    pub fn engine_as<T: Engine>(&mut self, id: EngineId) -> Option<&mut T> {
        let e: &mut dyn Any = self.engines.get_mut(id.0 as usize)?.as_mut();
        e.downcast_mut::<T>()
    }

    fn deliver(&mut self, ev: &Event) -> Result<bool, EngineError> {
        let idx = ev.target as usize;
        let Some(engine) = self.engines.get_mut(idx) else {
            return Ok(false);
        };
        let mut cx = SyncCtx {
            sched: &mut self.sched,
            now: self.now,
            id: EngineId(ev.target),
            stop: false,
            peers_quiescent: false,
        };
        engine.on_event(ev, &mut cx)?;
        Ok(cx.stop)
    }

    /// Run until `t_end`, a breakpoint, a host event or a stop request.
    pub fn run_until(&mut self, t_end: SimTime) -> Result<Stop, EngineError> {
        loop {
            if self.breakpoints.remove(&self.now) {
                return Ok(Stop::Breakpoint(self.now));
            }
            while let Some(ev) = self.sched.pop_due(self.now) {
                self.stats.events += 1;
                if ev.target == HOST {
                    return Ok(Stop::Host(ev));
                }
                if self.deliver(&ev)? {
                    return Ok(Stop::Requested(EngineId(ev.target)));
                }
            }
            if self.now >= t_end {
                return Ok(Stop::Reached(self.now));
            }
            let start = self.now;
            let mut limit = start.saturating_add(self.quantum).min(t_end);
            if let Some(p) = self.sched.peek_time() {
                limit = limit.min(p);
            }
            if let Some(&b) = self.breakpoints.range(SimTime(start.0 + 1)..).next() {
                limit = limit.min(b);
            }
            for e in &self.engines {
                if let Some(t) = e.next_event().filter(|&t| t > start) {
                    limit = limit.min(t);
                }
            }
            let peers_quiescent = self.engines.iter().all(|e| e.is_quiescent());
            let mut stop = None;
            for (i, e) in self.engines.iter_mut().enumerate() {
                if e.local_time() >= limit {
                    continue;
                }
                let mut cx = SyncCtx {
                    sched: &mut self.sched,
                    now: start,
                    id: EngineId(i as u32),
                    stop: false,
                    peers_quiescent,
                };
                let reached = e.advance_to(limit, &mut cx)?;
                if cx.stop {
                    stop.get_or_insert(EngineId(i as u32));
                }
                if reached <= start {
                    return Err(EngineError {
                        engine: e.name().to_owned(),
                        at: start,
                        message: format!("advance_to({}) returned {} ps without progress", limit.0, reached.0),
                    });
                }
                if reached < limit {
                    limit = reached;
                    self.stats.early_stops += 1;
                }
            }
            self.now = limit;
            self.sched.advance_clock(limit);
            self.stats.slices += 1;
            for e in &self.engines {
                let lead = SimTime(e.local_time().0.saturating_sub(limit.0));
                self.stats.max_skew = self.stats.max_skew.max(lead);
            }
            if let Some(id) = stop {
                return Ok(Stop::Requested(id));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clock that ticks every `period` and counts its ticks; it can stop
    /// early at a programmed time and records the events it receives.
    struct Ticker {
        t: SimTime,
        period: u64,
        ticks: u64,
        stop_at: Option<SimTime>,
        got: Vec<(SimTime, u32)>,
        post_to: Option<u32>,
    }

    impl Ticker {
        fn new(period: u64) -> Self {
            Self {
                t: SimTime::ZERO,
                period,
                ticks: 0,
                stop_at: None,
                got: Vec::new(),
                post_to: None,
            }
        }
    }

    impl Engine for Ticker {
        fn name(&self) -> &str {
            "ticker"
        }
        fn local_time(&self) -> SimTime {
            self.t
        }
        fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
            let target = match self.stop_at.take() {
                Some(s) if s < limit && s > self.t => {
                    if let Some(to) = self.post_to {
                        cx.schedule(s, to, 7);
                    }
                    s
                }
                other => {
                    self.stop_at = other;
                    limit
                }
            };
            self.ticks += target.0 / self.period - self.t.0 / self.period;
            self.t = target;
            Ok(target)
        }
        fn next_event(&self) -> Option<SimTime> {
            Some(SimTime((self.t.0 / self.period + 1) * self.period))
        }
        fn on_event(&mut self, ev: &Event, _cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
            self.got.push((ev.at, ev.code));
            Ok(())
        }
    }

    #[test]
    fn slices_end_at_events_breakpoints_and_wakeups() {
        let mut k = Kernel::new(SimTime::from_us(10));
        let a = k.add_engine(Box::new(Ticker::new(3_000_000)));
        k.schedule(SimTime::from_us(5), a.0, 1);
        k.add_breakpoint(SimTime::from_us(7));
        assert_eq!(
            k.run_until(SimTime::from_us(20)).unwrap(),
            Stop::Breakpoint(SimTime::from_us(7))
        );
        assert_eq!(k.engine_as::<Ticker>(a).unwrap().got, vec![(SimTime::from_us(5), 1)]);
        assert_eq!(
            k.run_until(SimTime::from_us(20)).unwrap(),
            Stop::Reached(SimTime::from_us(20))
        );
        let t = k.engine_as::<Ticker>(a).unwrap();
        assert_eq!(t.ticks, 6);
        assert_eq!(t.t, SimTime::from_us(20));
        // Slices: wake-ups every 3 us plus the event at 5 and breakpoint at 7.
        assert_eq!(k.stats().slices, 9);
    }

    #[test]
    fn early_stop_shrinks_the_slice_for_later_engines() {
        let mut k = Kernel::new(SimTime::from_us(100));
        let mut cpu = Ticker::new(u64::MAX);
        cpu.stop_at = Some(SimTime::from_us(30));
        let cpu = k.add_engine(Box::new(cpu));
        let mut analog = Ticker::new(u64::MAX);
        analog.post_to = Some(cpu.0);
        analog.stop_at = Some(SimTime::from_us(20));
        let analog = k.add_engine(Box::new(analog));
        // The CPU stops at 30 us, the analog engine then stops at 20 us
        // (a threshold crossing) and posts an event to the CPU, which is
        // ahead by 10 us and receives the event stamped 20 us.
        assert_eq!(
            k.run_until(SimTime::from_us(100)).unwrap(),
            Stop::Reached(SimTime::from_us(100))
        );
        assert_eq!(k.stats().early_stops, 2);
        assert_eq!(k.stats().max_skew, SimTime::from_us(10));
        assert_eq!(k.stats().slices, 2);
        assert_eq!(k.engine_as::<Ticker>(analog).unwrap().t, SimTime::from_us(100));
        assert_eq!(k.engine_as::<Ticker>(cpu).unwrap().got, vec![(SimTime::from_us(20), 7)]);
    }

    #[test]
    fn host_events_return_to_the_caller_in_order() {
        let mut k = Kernel::new(SimTime::from_us(1));
        k.add_engine(Box::new(Ticker::new(1_000)));
        k.schedule(SimTime::from_ns(2500), HOST, 2);
        k.schedule(SimTime::from_ns(1500), HOST, 1);
        let Stop::Host(e1) = k.run_until(SimTime::from_us(5)).unwrap() else {
            panic!("expected host event")
        };
        assert_eq!((e1.at, e1.code), (SimTime::from_ns(1500), 1));
        assert_eq!(k.now(), SimTime::from_ns(1500));
        let Stop::Host(e2) = k.run_until(SimTime::from_us(5)).unwrap() else {
            panic!("expected host event")
        };
        assert_eq!(e2.code, 2);
        assert_eq!(
            k.run_until(SimTime::from_us(5)).unwrap(),
            Stop::Reached(SimTime::from_us(5))
        );
        assert!(k.engine_as::<Ticker>(EngineId(5)).is_none());
    }

    #[test]
    fn engines_must_make_progress() {
        struct Stuck;
        impl Engine for Stuck {
            fn name(&self) -> &str {
                "stuck"
            }
            fn local_time(&self) -> SimTime {
                SimTime::ZERO
            }
            fn advance_to(&mut self, _limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
                Ok(cx.now())
            }
        }
        let mut k = Kernel::new(SimTime::from_us(1));
        k.add_engine(Box::new(Stuck));
        let e = k.run_until(SimTime::from_us(1)).unwrap_err();
        assert_eq!(e.engine, "stuck");
    }
}
