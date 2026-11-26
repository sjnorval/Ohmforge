//! UART console decoding (engine E5g, RFC 0004): an asynchronous serial
//! line read from a pin's logic level, 8 data bits, LSB first, no parity,
//! one stop bit (8N1), idle high. Each bit is read at its centre, timed from
//! the falling edge of the start bit, as a UART receiver does; a low stop bit
//! is a framing error.

/// A decoded byte or a framing error, with the time of its start edge (s).
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum UartEvent {
    /// A byte.
    Byte(f64, u8),
    /// The stop bit was low.
    FramingError(f64),
}

/// Streaming 8N1 decoder over (time, level) samples.
#[derive(Debug, Clone, PartialEq)]
pub struct UartDecoder {
    /// Bits per second.
    pub baud: f64,
    level: bool,
    /// Start-edge time and bits read so far (start bit = 0).
    frame: Option<(f64, u32, u16)>,
}

impl UartDecoder {
    /// A decoder at `baud`, line idle (high).
    pub fn new(baud: f64) -> Self {
        Self {
            baud,
            level: true,
            frame: None,
        }
    }

    /// Feed the line level `level` sampled at `t` (non-decreasing times);
    /// returns the bytes completed by it.
    pub fn feed(&mut self, t: f64, level: bool) -> Vec<UartEvent> {
        let mut out = Vec::new();
        let bit = 1.0 / self.baud;
        // Read every bit centre that `t` has passed with the level held until now.
        while let Some((start, n, data)) = self.frame {
            let centre = start + (f64::from(n) + 0.5) * bit;
            if t < centre {
                break;
            }
            let held = if t == centre { level } else { self.level };
            match n {
                0 if held => self.frame = None, // a glitch, not a start bit
                0 => self.frame = Some((start, 1, data)),
                1..=8 => self.frame = Some((start, n + 1, data | (u16::from(held) << (n - 1)))),
                _ => {
                    out.push(if held {
                        UartEvent::Byte(start, data as u8)
                    } else {
                        UartEvent::FramingError(start)
                    });
                    self.frame = None;
                }
            }
        }
        if self.frame.is_none() && self.level && !level {
            self.frame = Some((t, 0, 0));
        }
        self.level = level;
        out
    }

    /// The decoded bytes as text (non-UTF-8 bytes as U+FFFD).
    pub fn text(events: &[UartEvent]) -> String {
        let bytes: Vec<u8> = events
            .iter()
            .filter_map(|e| match e {
                UartEvent::Byte(_, b) => Some(*b),
                UartEvent::FramingError(_) => None,
            })
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The 8N1 line for `bytes` at `baud`, sampled every `dt` from t = 0.
    fn line(bytes: &[u8], baud: f64, dt: f64) -> Vec<(f64, bool)> {
        let bit = 1.0 / baud;
        let mut levels = vec![true; 2];
        for b in bytes {
            levels.push(false);
            levels.extend((0..8).map(|k| b >> k & 1 == 1));
            levels.push(true);
        }
        levels.extend([true; 2]);
        let end = levels.len() as f64 * bit;
        let n = (end / dt) as usize;
        (0..n)
            .map(|k| {
                let t = k as f64 * dt;
                (t, levels[((t / bit) as usize).min(levels.len() - 1)])
            })
            .collect()
    }

    fn decode(samples: &[(f64, bool)], baud: f64) -> Vec<UartEvent> {
        let mut d = UartDecoder::new(baud);
        samples.iter().flat_map(|&(t, l)| d.feed(t, l)).collect()
    }

    #[test]
    fn decodes_8n1_at_115200_from_coarse_samples() {
        // 8 samples per bit, as an analyser at 921.6 kHz would take.
        let baud = 115_200.0;
        let ev = decode(&line(b"ohmforge\n", baud, 1.0 / (8.0 * baud)), baud);
        assert_eq!(UartDecoder::text(&ev), "ohmforge\n");
        let UartEvent::Byte(t0, b'o') = ev[0] else {
            panic!("{ev:?}")
        };
        assert!((t0 - 2.0 / baud).abs() <= 1.0 / (8.0 * baud), "{t0}");
    }

    #[test]
    fn the_wrong_baud_gives_framing_errors_or_other_bytes() {
        let ev = decode(&line(b"U", 9600.0, 1e-6), 19_200.0);
        assert_ne!(UartDecoder::text(&ev), "U");
    }

    #[test]
    fn a_low_stop_bit_is_a_framing_error_and_a_short_glitch_is_ignored() {
        let baud = 9600.0;
        let bit = 1.0 / baud;
        let mut d = UartDecoder::new(baud);
        // Glitch shorter than half a bit.
        let mut ev = d.feed(0.0, false);
        ev.extend(d.feed(0.2 * bit, true));
        ev.extend(d.feed(2.0 * bit, true));
        assert!(ev.is_empty(), "{ev:?}");
        // Start at 3 bits, then all low through the stop bit.
        ev.extend(d.feed(3.0 * bit, false));
        ev.extend(d.feed(20.0 * bit, false));
        assert_eq!(ev, vec![UartEvent::FramingError(3.0 * bit)]);
    }
}
