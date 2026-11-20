//! Ohmforge simulation kernel.
//!
//! Owns virtual time and the event queue. Every other engine (CPU cores,
//! peripherals, digital and analog solvers) advances under this scheduler, so
//! runs are deterministic and replayable.
//!
//! * [`Scheduler`]: the picosecond clock and event queue.
//! * [`Engine`]: the trait every engine implements (`advance_to(t)`), and
//!   [`Kernel`], which advances a set of engines in time slices under one
//!   scheduler, delivering events and stopping at breakpoints.
//! * [`ValueCode`]: a real value (such as a sampled voltage) carried in an
//!   event code, so engines can exchange measurements without depending
//!   on each other.

mod engine;

pub use engine::{Engine, EngineError, EngineId, HOST, Kernel, KernelStats, Stop, SyncCtx};

use std::cmp::Reverse;
use std::collections::BinaryHeap;

/// Simulation time in picoseconds. `u64` covers about 213 days.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct SimTime(pub u64);

impl SimTime {
    /// Time zero.
    pub const ZERO: Self = Self(0);
    /// From nanoseconds.
    pub const fn from_ns(ns: u64) -> Self {
        Self(ns * 1_000)
    }
    /// From microseconds.
    pub const fn from_us(us: u64) -> Self {
        Self(us * 1_000_000)
    }
    /// From milliseconds.
    pub const fn from_ms(ms: u64) -> Self {
        Self(ms * 1_000_000_000)
    }
    /// As seconds (lossy), for display and analog maths.
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 * 1e-12
    }
    /// From seconds, rounded to the nearest picosecond (negative and NaN
    /// give zero, values beyond the range saturate).
    pub fn from_secs_f64(s: f64) -> Self {
        let ps = (s * 1e12).round();
        if ps.is_nan() || ps <= 0.0 {
            Self::ZERO
        } else if ps >= u64::MAX as f64 {
            Self(u64::MAX)
        } else {
            Self(ps as u64)
        }
    }
    /// `self + d`, saturating at the end of the range.
    pub const fn saturating_add(self, d: SimTime) -> Self {
        Self(self.0.saturating_add(d.0))
    }
}

/// A family of event codes that carry a real value: `base` plus the value
/// in units of `lsb`, rounded and clamped to `0..=mask`. Codes with other
/// bits outside `mask` than `base` belong to someone else.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ValueCode {
    /// The fixed bits (must not overlap `mask`).
    pub base: u32,
    /// The value field (contiguous low bits).
    pub mask: u32,
    /// Value of one count.
    pub lsb: f64,
}

impl ValueCode {
    /// The code for `value` (negative and NaN give 0, large values
    /// saturate at `mask`).
    pub fn encode(&self, value: f64) -> u32 {
        let n = (value / self.lsb).round();
        let n = if n.is_nan() || n <= 0.0 {
            0
        } else if n >= f64::from(self.mask) {
            self.mask
        } else {
            n as u32
        };
        self.base | n
    }

    /// The value in `code`, if it belongs to this family.
    pub fn decode(&self, code: u32) -> Option<f64> {
        (code & !self.mask == self.base).then(|| f64::from(code & self.mask) * self.lsb)
    }

    /// The largest value that can be carried.
    pub fn max(&self) -> f64 {
        f64::from(self.mask) * self.lsb
    }
}

/// A scheduled event: a target identifier and an opaque payload code.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct Event {
    /// When the event fires.
    pub at: SimTime,
    /// Insertion order, used to break ties deterministically.
    pub seq: u64,
    /// Which engine or component receives it.
    pub target: u32,
    /// Engine-defined event code.
    pub code: u32,
}

/// Deterministic discrete-event scheduler.
#[derive(Debug, Default)]
pub struct Scheduler {
    now: SimTime,
    seq: u64,
    queue: BinaryHeap<Reverse<Event>>,
}

impl Scheduler {
    /// New scheduler at time zero.
    pub fn new() -> Self {
        Self::default()
    }
    /// Current simulation time.
    pub fn now(&self) -> SimTime {
        self.now
    }
    /// Schedule an event; events in the past are clamped to `now`.
    pub fn schedule(&mut self, at: SimTime, target: u32, code: u32) {
        let at = at.max(self.now);
        self.seq += 1;
        self.queue.push(Reverse(Event {
            at,
            seq: self.seq,
            target,
            code,
        }));
    }
    /// Pop the next event and advance time to it.
    pub fn next_event(&mut self) -> Option<Event> {
        let Reverse(ev) = self.queue.pop()?;
        self.now = ev.at;
        Some(ev)
    }
    /// Time of the next pending event, if any (used as an analog breakpoint).
    pub fn peek_time(&self) -> Option<SimTime> {
        self.queue.peek().map(|Reverse(e)| e.at)
    }
    /// Pop the next event if it is due at or before `t` (time advances to
    /// the event, never backwards).
    pub fn pop_due(&mut self, t: SimTime) -> Option<Event> {
        if self.peek_time()? <= t {
            self.next_event()
        } else {
            None
        }
    }
    /// Move the clock forward to `t` without firing events. Clamped so that
    /// time never goes backwards or past a pending event.
    pub fn advance_clock(&mut self, t: SimTime) {
        let t = self.peek_time().map_or(t, |p| t.min(p));
        self.now = self.now.max(t);
    }
    /// Number of pending events.
    pub fn pending(&self) -> usize {
        self.queue.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_fire_in_time_then_insertion_order() {
        let mut s = Scheduler::new();
        s.schedule(SimTime::from_ns(5), 1, 0);
        s.schedule(SimTime::from_ns(1), 2, 0);
        s.schedule(SimTime::from_ns(5), 3, 0);
        let order: Vec<u32> = std::iter::from_fn(|| s.next_event()).map(|e| e.target).collect();
        assert_eq!(order, vec![2, 1, 3]);
        assert_eq!(s.now(), SimTime::from_ns(5));
    }

    #[test]
    fn value_codes_round_trip_with_rounding_and_clamping() {
        let vc = ValueCode {
            base: 0x8500_0000,
            mask: 0x00ff_ffff,
            lsb: 1e-6,
        };
        assert_eq!(vc.encode(1.2345674), 0x8500_0000 | 1_234_567);
        assert_eq!(vc.decode(vc.encode(1.5)), Some(1.5));
        assert_eq!(vc.encode(-0.2), 0x8500_0000);
        assert_eq!(vc.encode(f64::NAN), 0x8500_0000);
        assert_eq!(vc.encode(100.0), 0x85ff_ffff);
        assert!((vc.max() - 16.777215).abs() < 1e-9);
        assert_eq!(vc.decode(0x8600_0001), None, "another family");
    }

    #[test]
    fn seconds_round_trip_and_clock_clamps() {
        assert_eq!(SimTime::from_secs_f64(1.5e-6), SimTime::from_ns(1500));
        assert_eq!(SimTime::from_secs_f64(-1.0), SimTime::ZERO);
        assert_eq!(SimTime::from_ms(2).as_secs_f64(), 2e-3);
        let mut s = Scheduler::new();
        s.schedule(SimTime::from_ns(10), 1, 0);
        s.advance_clock(SimTime::from_ns(50));
        assert_eq!(s.now(), SimTime::from_ns(10), "clock stops at the pending event");
        assert!(s.pop_due(SimTime::from_ns(9)).is_none());
        assert_eq!(s.pop_due(SimTime::from_ns(10)).map(|e| e.target), Some(1));
        s.advance_clock(SimTime::from_ns(5));
        assert_eq!(s.now(), SimTime::from_ns(10), "never backwards");
    }
}
