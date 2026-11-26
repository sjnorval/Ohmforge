//! The digital bench (engine E5g): UART consoles and logic analysers fed
//! from the pin voltages (`d(REF.pin)`) of a running circuit, captured on
//! one step grid.

use crate::Signal;
use crate::logic::{LogicAnalyser, LogicInput};
use crate::service::{ProbeError, Probeable};
use crate::uart::{UartDecoder, UartEvent};

/// The finest step a logic analyser is captured at (s). A real profile's
/// clock can be faster (the Saleae Logic 8 samples at up to 100 MS/s);
/// edges are then put on its clock from captures this far apart.
pub const MIN_LOGIC_STEP: f64 = 1e-6;

/// A UART console decoding one TX line (8N1) through an ideal input.
#[derive(Debug, Clone)]
pub struct UartConsole {
    /// Instrument ref (`XU1`).
    pub reference: String,
    /// The TX line, `d(REF.pin)`.
    pub signal: Signal,
    /// Baud rate the console listens at.
    pub baud: f64,
    input: LogicInput,
    decoder: UartDecoder,
    /// Bytes and framing errors so far.
    pub events: Vec<UartEvent>,
}

impl UartConsole {
    /// A console on `signal` at `baud`.
    pub fn new(reference: impl Into<String>, signal: Signal, baud: f64) -> Self {
        Self {
            reference: reference.into(),
            signal,
            baud,
            input: LogicInput::ideal(),
            decoder: UartDecoder::new(baud),
            events: Vec::new(),
        }
    }

    /// The text received so far (lossy UTF-8).
    pub fn text(&self) -> String {
        UartDecoder::text(&self.events)
    }

    /// Framing errors so far.
    pub fn framing_errors(&self) -> usize {
        self.events
            .iter()
            .filter(|e| matches!(e, UartEvent::FramingError(_)))
            .count()
    }
}

/// Consoles and analysers captured together.
#[derive(Debug, Clone, Default)]
pub struct DigitalBench {
    /// UART consoles.
    pub consoles: Vec<UartConsole>,
    /// Logic analysers, by instrument ref.
    pub analysers: Vec<(String, LogicAnalyser)>,
}

impl DigitalBench {
    /// Nothing to capture.
    pub fn is_empty(&self) -> bool {
        self.consoles.is_empty() && self.analysers.is_empty()
    }

    /// The capture step (s): an eighth of the fastest console's bit time,
    /// and each analyser's clock but no finer than [`MIN_LOGIC_STEP`].
    pub fn step(&self) -> Option<f64> {
        let consoles = self.consoles.iter().map(|c| 1.0 / (8.0 * c.baud));
        let analysers = self
            .analysers
            .iter()
            .map(|(_, a)| a.period.unwrap_or(MIN_LOGIC_STEP).max(MIN_LOGIC_STEP));
        consoles.chain(analysers).reduce(f64::min)
    }

    /// Capture every console and analyser from `src` now.
    pub fn capture<P: Probeable + ?Sized>(&mut self, src: &mut P) -> Result<(), ProbeError> {
        let now = src.now_secs();
        for c in &mut self.consoles {
            let Signal::Digital { refdes, pin } = &c.signal else {
                return Err(ProbeError::Unresolved(c.signal.clone()));
            };
            let level = c.input.feed(src.pin_volts(refdes, pin)?);
            let events = c.decoder.feed(now, level);
            c.events.extend(events);
        }
        for (_, a) in &mut self.analysers {
            a.capture(src)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A TX line sending 'A' (0x41) at 9600 Bd from 100 µs, idle high at 3.3 V.
    struct Line {
        t: f64,
    }

    impl Probeable for Line {
        fn now_secs(&self) -> f64 {
            self.t
        }
        fn sample(&mut self, s: &Signal) -> Result<f64, ProbeError> {
            Err(ProbeError::Unresolved(s.clone()))
        }
        fn pin_volts(&mut self, _: &str, _: &str) -> Result<f64, ProbeError> {
            let bit = ((self.t - 100e-6) * 9600.0).floor();
            let high = match bit as i64 {
                0 => false,
                k @ 1..=8 => 0x41u8 >> (k - 1) & 1 != 0,
                _ => true,
            };
            Ok(if high { 3.3 } else { 0.0 })
        }
    }

    #[test]
    fn a_console_and_an_analyser_share_the_grid() {
        let d = || "d(U1.GPIO21)".parse::<Signal>().unwrap();
        let mut bench = DigitalBench {
            consoles: vec![UartConsole::new("XU1", d(), 9600.0)],
            analysers: vec![("XLA1".into(), LogicAnalyser::ideal(vec![d()]).unwrap())],
        };
        let step = bench.step().unwrap();
        assert_eq!(step, MIN_LOGIC_STEP, "the ideal analyser is finer than 1/8 bit");
        let mut line = Line { t: 0.0 };
        while line.t < 2e-3 {
            bench.capture(&mut line).unwrap();
            line.t += step;
        }
        assert_eq!(bench.consoles[0].text(), "A");
        assert_eq!(bench.consoles[0].framing_errors(), 0);
        // 0x41 framed: start, 1, 0, 0, 0, 0, 0, 1, 0, stop: 6 edges.
        assert_eq!(bench.analysers[0].1.channels[0].edges.len(), 6);
    }

    #[test]
    fn an_empty_bench_has_no_step() {
        assert!(DigitalBench::default().is_empty());
        assert_eq!(DigitalBench::default().step(), None);
    }
}
