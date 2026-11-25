//! The measurement service (RFC 0004): watched signals sampled from a
//! [`Probeable`] source on a [`SamplePlan`] into bounded rings.
//!
//! The run loop owns time: after each engine step it calls
//! [`MeasurementService::on_step`], saying whether an exact event (an LEDC
//! edge today) was hit. A sample is recorded when the grid is due, or on the
//! event when the plan asks for events. Every channel is sampled at the same
//! instant, or none is (a failed probe records nothing).

use crate::ring::{SAMPLE_STEP, SampleClock, TRACE_WINDOW, TraceBuf};
use crate::signal::Signal;

/// Why a signal could not be sampled.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProbeError {
    /// The net, component, pin or reading is not in the circuit.
    #[error("`{0}` is not in this circuit")]
    Unresolved(Signal),
    /// The signal has no analog value (for example a digital pin state read as a number).
    #[error("`{0}` has no analog value")]
    NotAnalog(Signal),
}

/// A running circuit the service can sample. The engine `Session` implements
/// it from E5b; tests use a fake.
pub trait Probeable {
    /// Simulation time since the circuit started (seconds).
    fn now_secs(&self) -> f64;
    /// The value of `signal` at [`Self::now_secs`] (V, A or the reading's unit).
    fn sample(&mut self, signal: &Signal) -> Result<f64, ProbeError>;
}

/// When the service records samples.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SamplePlan {
    /// A fixed grid with this spacing (seconds).
    Every(f64),
    /// The grid, plus every exact event instant the run loop reports.
    EveryAndEvents(f64),
}

impl SamplePlan {
    /// The grid spacing (seconds).
    pub fn step(self) -> f64 {
        match self {
            Self::Every(s) | Self::EveryAndEvents(s) => s,
        }
    }
}

impl Default for SamplePlan {
    /// Today's desktop scope: the 50 µs grid plus exact events.
    fn default() -> Self {
        Self::EveryAndEvents(SAMPLE_STEP)
    }
}

/// Handle to a watched signal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ChannelId(usize);

/// One watched signal and its samples.
#[derive(Debug, Clone)]
pub struct Channel {
    /// What is sampled.
    pub signal: Signal,
    /// Recorded `(t, value)` samples.
    pub ring: TraceBuf,
}

/// Samples watched signals into bounded rings.
#[derive(Debug, Clone)]
pub struct MeasurementService {
    plan: SamplePlan,
    clock: SampleClock,
    channels: Vec<Option<Channel>>,
}

impl MeasurementService {
    /// An empty service whose grid starts at `t = 0`.
    pub fn new(plan: SamplePlan) -> Self {
        Self {
            plan,
            clock: SampleClock::new(0.0, plan.step()),
            channels: Vec::new(),
        }
    }

    /// The sampling plan.
    pub fn plan(&self) -> SamplePlan {
        self.plan
    }

    /// Watch `signal`, keeping `window` seconds of samples at the plan's step.
    /// Watching a signal that is already watched returns the same channel.
    pub fn watch(&mut self, signal: Signal, window: f64) -> ChannelId {
        if let Some((id, _)) = self.channels().find(|(_, c)| c.signal == signal) {
            return id;
        }
        self.channels.push(Some(Channel {
            signal,
            ring: TraceBuf::with_window(window, self.clock.step()),
        }));
        ChannelId(self.channels.len() - 1)
    }

    /// Watch `signal` with the default scope retention ([`TRACE_WINDOW`]).
    pub fn watch_default(&mut self, signal: Signal) -> ChannelId {
        self.watch(signal, TRACE_WINDOW)
    }

    /// Stop watching a channel; its id is not reused.
    pub fn unwatch(&mut self, id: ChannelId) {
        if let Some(c) = self.channels.get_mut(id.0) {
            *c = None;
        }
    }

    /// The channel for `id`, if it is still watched.
    pub fn channel(&self, id: ChannelId) -> Option<&Channel> {
        self.channels.get(id.0).and_then(Option::as_ref)
    }

    /// Watched channels in watch order.
    pub fn channels(&self) -> impl Iterator<Item = (ChannelId, &Channel)> + '_ {
        self.channels
            .iter()
            .enumerate()
            .filter_map(|(i, c)| c.as_ref().map(|c| (ChannelId(i), c)))
    }

    /// Call after each engine step. Records one sample on every channel when
    /// the grid is due, or when `event_hit` and the plan samples events.
    /// Returns whether a sample was recorded.
    pub fn on_step<P: Probeable + ?Sized>(&mut self, src: &mut P, event_hit: bool) -> Result<bool, ProbeError> {
        let now = src.now_secs();
        let grid = self.clock.take(now);
        let event = event_hit && matches!(self.plan, SamplePlan::EveryAndEvents(_));
        if !(grid || event) {
            return Ok(false);
        }
        let mut values = Vec::with_capacity(self.channels.len());
        for c in self.channels.iter().flatten() {
            values.push(src.sample(&c.signal)?);
        }
        for (c, v) in self.channels.iter_mut().flatten().zip(values) {
            c.ring.push(now, v);
        }
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;

    /// A fake circuit: fixed values per signal at a settable time.
    struct Fake {
        t: f64,
        values: BTreeMap<Signal, f64>,
    }

    impl Probeable for Fake {
        fn now_secs(&self) -> f64 {
            self.t
        }
        fn sample(&mut self, signal: &Signal) -> Result<f64, ProbeError> {
            self.values
                .get(signal)
                .copied()
                .ok_or_else(|| ProbeError::Unresolved(signal.clone()))
        }
    }

    fn sig(s: &str) -> Signal {
        s.parse().expect("signal")
    }

    fn fake() -> Fake {
        let mut values = BTreeMap::new();
        values.insert(sig("i(R1)"), 4e-3);
        values.insert(sig("v(vdd)"), 3.3);
        Fake { t: 0.0, values }
    }

    #[test]
    fn samples_every_channel_on_the_grid_only() {
        let mut svc = MeasurementService::new(SamplePlan::Every(50e-6));
        let i = svc.watch_default(sig("i(R1)"));
        let v = svc.watch_default(sig("v(vdd)"));
        let mut src = fake();
        // t = 0 is on the grid.
        assert!(svc.on_step(&mut src, false).unwrap());
        // 10 µs later: not due, and events are not sampled by this plan.
        src.t = 10e-6;
        assert!(!svc.on_step(&mut src, true).unwrap());
        src.t = 50e-6;
        assert!(svc.on_step(&mut src, false).unwrap());
        let ci = svc.channel(i).unwrap();
        assert_eq!(ci.ring.to_vec(), vec![(0.0, 4e-3), (50e-6, 4e-3)]);
        assert_eq!(svc.channel(v).unwrap().ring.to_vec(), vec![(0.0, 3.3), (50e-6, 3.3)]);
    }

    #[test]
    fn events_are_sampled_when_the_plan_asks() {
        let mut svc = MeasurementService::new(SamplePlan::EveryAndEvents(50e-6));
        let i = svc.watch_default(sig("i(R1)"));
        let mut src = fake();
        assert!(svc.on_step(&mut src, false).unwrap());
        src.t = 12e-6;
        assert!(!svc.on_step(&mut src, false).unwrap());
        assert!(svc.on_step(&mut src, true).unwrap());
        assert_eq!(svc.channel(i).unwrap().ring.len(), 2);
    }

    #[test]
    fn grid_matches_the_desktop_sample_clock() {
        // Same decisions as `SampleClock` on the same times (what SessionSim does today).
        let mut svc = MeasurementService::new(SamplePlan::Every(SAMPLE_STEP));
        svc.watch_default(sig("v(vdd)"));
        let mut clock = SampleClock::new(0.0, SAMPLE_STEP);
        let mut src = fake();
        for k in 0..2000 {
            src.t = k as f64 * 7.3e-6;
            assert_eq!(
                svc.on_step(&mut src, false).unwrap(),
                clock.take(src.t),
                "t = {}",
                src.t
            );
        }
    }

    #[test]
    fn watching_twice_shares_a_channel_and_unwatch_drops_it() {
        let mut svc = MeasurementService::new(SamplePlan::default());
        assert_eq!(svc.plan(), SamplePlan::EveryAndEvents(SAMPLE_STEP));
        let a = svc.watch_default(sig("v(vdd)"));
        let b = svc.watch(sig("v( vdd )"), 1.0);
        assert_eq!(a, b);
        let c = svc.watch_default(sig("i(R1)"));
        assert_ne!(a, c);
        svc.unwatch(a);
        assert!(svc.channel(a).is_none());
        assert_eq!(svc.channels().map(|(id, _)| id).collect::<Vec<_>>(), vec![c]);
        // A new watch gets a fresh id.
        let d = svc.watch_default(sig("v(vdd)"));
        assert_ne!(d, a);
    }

    #[test]
    fn retention_follows_the_requested_window() {
        let mut svc = MeasurementService::new(SamplePlan::Every(1e-3));
        let id = svc.watch(sig("v(vdd)"), 0.010);
        let mut src = fake();
        for k in 0..100 {
            src.t = k as f64 * 1e-3;
            svc.on_step(&mut src, false).unwrap();
        }
        let ring = &svc.channel(id).unwrap().ring;
        let pts = ring.to_vec();
        assert!(pts.first().unwrap().0 >= 0.099 - 0.010 - 1e-12, "{pts:?}");
        assert_eq!(pts.last().unwrap().0, 0.099);
    }

    #[test]
    fn an_unresolved_signal_records_nothing() {
        let mut svc = MeasurementService::new(SamplePlan::Every(50e-6));
        let i = svc.watch_default(sig("i(R1)"));
        svc.watch_default(sig("v(nowhere)"));
        let mut src = fake();
        assert_eq!(
            svc.on_step(&mut src, false),
            Err(ProbeError::Unresolved(sig("v(nowhere)")))
        );
        assert!(svc.channel(i).unwrap().ring.is_empty());
    }
}
