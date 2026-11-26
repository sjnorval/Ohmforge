//! Logic analyser (engine E5g, RFC 0004): `d(REF.pin)` channels read the
//! pin's net voltage through a [`LogicInput`] (thresholds with the last state
//! held between them, as a real input does) and record edges on the
//! analyser's sample clock.
//!
//! The ideal profile uses one threshold, [`IDEAL_THRESHOLD`], and exact
//! times; the Saleae Logic 8 profile (`parts/instruments/saleae.logic8`) uses
//! VIL 0.6 V / VIH 1.2 V, holds the state between them (Saleae calls that
//! band undefined) and places each edge on the next tick of its sample clock.

use crate::profile::LogicProfile;
use crate::service::{ProbeError, Probeable};
use crate::signal::Signal;

/// The ideal input's single threshold (V): half of a 3.3 V rail.
pub const IDEAL_THRESHOLD: f64 = 1.65;

/// A digital input: high at or above `vih`, low at or below `vil`, and the
/// last state between them.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LogicInput {
    /// Highest voltage read as low.
    pub vil: f64,
    /// Lowest voltage read as high.
    pub vih: f64,
    state: bool,
}

impl LogicInput {
    /// An input with thresholds `vil` ≤ `vih`, starting low.
    pub fn new(vil: f64, vih: f64) -> Self {
        Self { vil, vih, state: false }
    }

    /// The ideal input: one threshold at [`IDEAL_THRESHOLD`].
    pub fn ideal() -> Self {
        Self::new(IDEAL_THRESHOLD, IDEAL_THRESHOLD)
    }

    /// The state after seeing `volts`.
    pub fn feed(&mut self, volts: f64) -> bool {
        if volts >= self.vih {
            self.state = true;
        } else if volts <= self.vil {
            self.state = false;
        }
        self.state
    }

    /// The current state.
    pub fn state(&self) -> bool {
        self.state
    }
}

/// One analyser channel.
#[derive(Debug, Clone, PartialEq)]
pub struct LogicChannel {
    /// The `d(REF.pin)` signal.
    pub signal: Signal,
    input: LogicInput,
    /// The first state seen and when (`None` before the first sample).
    pub initial: Option<(f64, bool)>,
    /// Edges: (time on the sample clock, new state).
    pub edges: Vec<(f64, bool)>,
}

impl LogicChannel {
    /// The state at time `t` (before the first sample: the initial state).
    pub fn state_at(&self, t: f64) -> Option<bool> {
        let (_, first) = self.initial?;
        Some(
            self.edges
                .iter()
                .take_while(|(te, _)| *te <= t)
                .last()
                .map_or(first, |(_, s)| *s),
        )
    }
}

/// A logic analyser over `d(..)` channels.
#[derive(Debug, Clone, PartialEq)]
pub struct LogicAnalyser {
    /// Sample period (s); `None` for exact (ideal) times.
    pub period: Option<f64>,
    /// Channels in watch order.
    pub channels: Vec<LogicChannel>,
}

/// Why an analyser could not be set up.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum LogicError {
    /// A channel is not a `d(REF.pin)` signal.
    #[error("`{0}` is not a digital signal d(REF.pin)")]
    NotDigital(Signal),
    /// More channels than the analyser has.
    #[error("{0} channels, but the analyser has {1}")]
    TooManyChannels(usize, usize),
    /// No listed rate at or below the request for this many channels.
    #[error("no sample rate at or below {0} Hz for {1} channels")]
    NoRate(f64, usize),
}

impl LogicAnalyser {
    /// An ideal analyser: one threshold, exact edge times.
    pub fn ideal(signals: Vec<Signal>) -> Result<Self, LogicError> {
        Self::with(signals, LogicInput::ideal(), None)
    }

    /// An analyser with `profile`'s thresholds at the highest rate it allows
    /// for this many channels at or below `requested` Hz.
    pub fn real(signals: Vec<Signal>, profile: &LogicProfile, requested: f64) -> Result<Self, LogicError> {
        let n = signals.len();
        if n > profile.channels {
            return Err(LogicError::TooManyChannels(n, profile.channels));
        }
        let rate = profile.rate(requested, n).ok_or(LogicError::NoRate(requested, n))?;
        Self::with(signals, profile.input(), Some(1.0 / rate))
    }

    fn with(signals: Vec<Signal>, input: LogicInput, period: Option<f64>) -> Result<Self, LogicError> {
        let channels = signals
            .into_iter()
            .map(|signal| match signal {
                Signal::Digital { .. } => Ok(LogicChannel {
                    signal,
                    input,
                    initial: None,
                    edges: Vec::new(),
                }),
                other => Err(LogicError::NotDigital(other)),
            })
            .collect::<Result<_, _>>()?;
        Ok(Self { period, channels })
    }

    /// `t` on the sample clock: the first tick at or after `t`.
    pub fn on_clock(&self, t: f64) -> f64 {
        match self.period {
            Some(p) => (t / p - 1e-9).ceil() * p,
            None => t,
        }
    }

    /// Sample every channel's pin voltage from `src` now.
    pub fn capture<P: Probeable + ?Sized>(&mut self, src: &mut P) -> Result<(), ProbeError> {
        let now = src.now_secs();
        let t = self.on_clock(now);
        for ch in &mut self.channels {
            let Signal::Digital { refdes, pin } = &ch.signal else {
                continue;
            };
            let v = src.pin_volts(refdes, pin)?;
            let before = ch.input.state();
            let s = ch.input.feed(v);
            match ch.initial {
                None => ch.initial = Some((t, s)),
                Some(_) if s != before => ch.edges.push((t, s)),
                Some(_) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::logics;

    struct Pin {
        t: f64,
        v: f64,
    }

    impl Probeable for Pin {
        fn now_secs(&self) -> f64 {
            self.t
        }
        fn sample(&mut self, s: &Signal) -> Result<f64, ProbeError> {
            Err(ProbeError::NotAnalog(s.clone()))
        }
        fn pin_volts(&mut self, _refdes: &str, _pin: &str) -> Result<f64, ProbeError> {
            Ok(self.v)
        }
    }

    fn gpio() -> Signal {
        "d(U1.GPIO8)".parse().unwrap()
    }

    fn saleae() -> &'static LogicProfile {
        logics().find(|p| p.id == "saleae.logic8").unwrap()
    }

    #[test]
    fn inputs_hold_their_state_between_the_thresholds() {
        let mut i = LogicInput::new(0.6, 1.2);
        assert!(!i.feed(0.9), "starts low and holds in the band");
        assert!(i.feed(1.2));
        assert!(i.feed(0.61), "holds high in the band");
        assert!(!i.feed(0.6));
        let mut ideal = LogicInput::ideal();
        assert!(!ideal.feed(1.64) && ideal.feed(1.65));
    }

    #[test]
    fn saleae_rates_follow_the_channel_count() {
        let p = saleae();
        assert_eq!(p.rate(100e6, 3), Some(100e6));
        assert_eq!(p.rate(100e6, 4), Some(50e6));
        assert_eq!(p.rate(100e6, 7), Some(40e6));
        assert_eq!(p.rate(100e6, 8), Some(25e6));
        assert_eq!(p.rate(3e6, 1), Some(2e6));
        assert_eq!(p.rate(0.5e6, 1), None);
        let many = vec![gpio(); 9];
        assert_eq!(
            LogicAnalyser::real(many, p, 25e6),
            Err(LogicError::TooManyChannels(9, 8))
        );
    }

    #[test]
    fn edges_land_on_the_sample_clock() {
        let mut la = LogicAnalyser::real(vec![gpio()], saleae(), 1e6).unwrap();
        assert_eq!(la.period, Some(1e-6));
        let mut pin = Pin { t: 0.0, v: 0.0 };
        la.capture(&mut pin).unwrap();
        // 0.9 V is in the undefined band: still low.
        pin.t = 1.2e-6;
        pin.v = 0.9;
        la.capture(&mut pin).unwrap();
        // 3.3 V at 2.3 µs is seen on the 3 µs tick.
        pin.t = 2.3e-6;
        pin.v = 3.3;
        la.capture(&mut pin).unwrap();
        pin.t = 5.0e-6;
        pin.v = 0.2;
        la.capture(&mut pin).unwrap();
        let ch = &la.channels[0];
        assert_eq!(ch.initial, Some((0.0, false)));
        assert_eq!(ch.edges.len(), 2);
        assert!((ch.edges[0].0 - 3e-6).abs() < 1e-15 && ch.edges[0].1, "{:?}", ch.edges);
        assert!((ch.edges[1].0 - 5e-6).abs() < 1e-15 && !ch.edges[1].1, "{:?}", ch.edges);
        assert_eq!(ch.state_at(4e-6), Some(true));
        assert_eq!(ch.state_at(1e-6), Some(false));
    }

    #[test]
    fn ideal_times_are_exact_and_channels_must_be_digital() {
        let mut la = LogicAnalyser::ideal(vec![gpio()]).unwrap();
        let mut pin = Pin { t: 0.0, v: 0.0 };
        la.capture(&mut pin).unwrap();
        pin.t = 2.345e-6;
        pin.v = 3.3;
        la.capture(&mut pin).unwrap();
        assert_eq!(la.channels[0].edges, vec![(2.345e-6, true)]);
        let v: Signal = "v(vdd)".parse().unwrap();
        assert_eq!(LogicAnalyser::ideal(vec![v.clone()]), Err(LogicError::NotDigital(v)));
    }
}
