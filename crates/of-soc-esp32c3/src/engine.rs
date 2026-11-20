//! [`Esp32c3`] as an [`of_core::Engine`].
//!
//! The hart runs ahead freely until a peripheral access (a sync point).
//! Pin changes on linked pins become kernel events at the hart time of the
//! instruction, and the engine returns early there so every other engine
//! sees the change at exactly that time. Register it before the analog
//! engine.

use of_core::{Engine, EngineError, SimTime, SyncCtx};
use of_cpu_riscv::{HaltReason, RunExit};

use crate::gpio::PinOut;
use crate::soc::Esp32c3;

/// Send changes on `pin` to engine `target` as [`pin_code`] events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinLink {
    /// GPIO number.
    pub pin: u8,
    /// Receiving engine (its [`of_core::EngineId`] number).
    pub target: u32,
}

/// Event code for "pin `pin` now does `out`": bits 15:8 pin, 3:2 drive
/// strength, 1:0 level (0 low, 1 high, 2 high-impedance).
pub fn pin_code(pin: u8, out: PinOut) -> u32 {
    let level = match out.level {
        Some(false) => 0,
        Some(true) => 1,
        None => 2,
    };
    (u32::from(pin) << 8) | (u32::from(out.strength & 3) << 2) | level
}

/// Inverse of [`pin_code`].
pub fn decode_pin_code(code: u32) -> Option<(u8, PinOut)> {
    let level = match code & 3 {
        0 => Some(false),
        1 => Some(true),
        2 => None,
        _ => return None,
    };
    (code >> 16 == 0).then_some(())?;
    Some((
        (code >> 8) as u8,
        PinOut {
            level,
            strength: ((code >> 2) & 3) as u8,
        },
    ))
}

/// Every event code a pin can produce, for registering analog actions.
pub fn all_pin_codes(pin: u8) -> impl Iterator<Item = (u32, PinOut)> {
    (0..4u8).flat_map(move |strength| {
        [Some(false), Some(true), None].into_iter().map(move |level| {
            let out = PinOut { level, strength };
            (pin_code(pin, out), out)
        })
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pin_codes_round_trip_and_are_distinct() {
        let mut seen = std::collections::BTreeSet::new();
        for pin in 0..22 {
            for (code, out) in all_pin_codes(pin) {
                assert_eq!(decode_pin_code(code), Some((pin, out)));
                assert!(seen.insert(code));
            }
        }
        assert_eq!(seen.len(), 22 * 12);
        assert_eq!(decode_pin_code(3), None);
    }
}

/// The ESP32-C3 engine.
#[derive(Debug)]
pub struct Esp32c3Engine {
    name: String,
    /// The chip.
    pub soc: Esp32c3,
    links: Vec<PinLink>,
    sent: usize,
}

impl Esp32c3Engine {
    /// Wrap a chip (firmware already loaded).
    pub fn new(name: &str, soc: Esp32c3) -> Self {
        Self {
            name: name.to_owned(),
            soc,
            links: Vec::new(),
            sent: 0,
        }
    }

    /// Forward changes of `link.pin` as events to `link.target`.
    pub fn link_pin(&mut self, link: PinLink) {
        self.links.push(link);
    }

    fn err(&self, message: String) -> EngineError {
        EngineError {
            engine: self.name.clone(),
            at: self.soc.hart.local_time(),
            message,
        }
    }
}

impl Engine for Esp32c3Engine {
    fn name(&self) -> &str {
        &self.name
    }

    fn local_time(&self) -> SimTime {
        self.soc.hart.local_time()
    }

    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        loop {
            let exit = self.soc.run_slice(limit);
            let edges = &self.soc.pin_edges()[self.sent..];
            self.sent = self.soc.pin_edges().len();
            let mut sent_any = false;
            for e in edges {
                for l in self.links.iter().filter(|l| l.pin == e.pin) {
                    cx.schedule(e.at, l.target, pin_code(e.pin, e.out));
                    sent_any = true;
                }
            }
            let t = self.soc.hart.local_time();
            match exit {
                RunExit::SyncRequested(_) if sent_any && t > cx.now() => return Ok(t),
                RunExit::SyncRequested(_) if t < limit => {}
                RunExit::SyncRequested(_) | RunExit::BudgetExhausted => return Ok(t.max(limit)),
                RunExit::Deferred(_) => return Ok(t),
                RunExit::WaitingForInterrupt => {
                    self.soc.hart.idle_until(limit);
                    return Ok(limit);
                }
                RunExit::Halted(HaltReason::Requested) | RunExit::Breakpoint(_) => {
                    return Err(self.err(format!("stopped for the debugger at {:#010x}", self.soc.hart.pc())));
                }
                RunExit::Halted(HaltReason::Ebreak { pc }) => {
                    return Err(self.err(format!("ebreak at {pc:#010x}")));
                }
            }
        }
    }
}
