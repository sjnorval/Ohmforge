//! Ohmforge simulation kernel.
//!
//! Owns virtual time and the event queue. Every other engine (CPU cores,
//! peripherals, digital and analog solvers) advances under this scheduler, so
//! runs are deterministic and replayable.

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
    /// As seconds (lossy), for display and analog maths.
    pub fn as_secs_f64(self) -> f64 {
        self.0 as f64 * 1e-12
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
}
