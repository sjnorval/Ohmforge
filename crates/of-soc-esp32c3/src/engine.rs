//! [`Esp32c3`] as an [`of_core::Engine`].
//!
//! Outputs: the hart runs ahead freely until a peripheral access (a sync
//! point). Pin changes on linked pins become kernel events at the hart time
//! of the instruction, and the engine returns early there so every other
//! engine sees the change at exactly that time. Register it before the
//! analog engine.
//!
//! Inputs: pins linked with [`Esp32c3Engine::link_input`] take their pad
//! level from [`input_code`] events (for example analog threshold watches
//! from [`input_watches`]). Input reads are exact in time:
//!
//! * a load from `GPIO_IN` or the GPIO interrupt status registers made
//!   later than the start of the current slice (the last time inputs are
//!   known) is deferred: the engine stops before the instruction, the other
//!   engines catch up and deliver their events, and the load runs in the
//!   next slice;
//! * a hart sleeping in `wfi` only claims the slice; an input event wakes
//!   it at the event's exact time.
//!
//! A hart that is busy (not reading inputs, not sleeping) may be ahead of
//! an input event by up to one slice, so a pin interrupt can be taken that
//! much late; [`Esp32c3Engine::set_max_lead`] bounds it.
//!
//! PWM: a pin the LEDC drives changes at the exact time of the PWM edge
//! (the counter tick), not when the hart notices; the engine sends the
//! event with that time and returns there. Every edge is one event, so
//! the cost of a PWM run grows with its frequency (and the analog engine
//! restarts its integration at each edge).
//!
//! ADC: pads linked with [`Esp32c3Engine::link_adc`] are sampled when a
//! conversion starts: the engine sends [`adc_request_code`] at that time
//! to the analog engine and returns there; the analog engine answers at
//! once with the voltage in an [`adc_value_code`] event (see
//! `of_analog::AnalogEngine::sample_on`), which the conversion uses.

use of_core::{Engine, EngineError, Event, SimTime, SyncCtx, ValueCode};
use of_cpu_riscv::{HaltReason, RunExit};

use crate::gpio::{PinOut, VIH_FRACTION, VIL_FRACTION};
use crate::soc::Esp32c3;

/// Send changes on `pin` to engine `target` as [`pin_code`] events.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinLink {
    /// GPIO number.
    pub pin: u8,
    /// Receiving engine (its [`of_core::EngineId`] number).
    pub target: u32,
}

/// Event code for "pin `pin` now does `out`": bits 15:8 pin, 5 pull-down,
/// 4 pull-up, 3:2 drive strength, 1:0 level (0 low, 1 high, 2
/// high-impedance).
pub fn pin_code(pin: u8, out: PinOut) -> u32 {
    let level = match out.level {
        Some(false) => 0,
        Some(true) => 1,
        None => 2,
    };
    (u32::from(pin) << 8)
        | (u32::from(out.pull_down) << 5)
        | (u32::from(out.pull_up) << 4)
        | (u32::from(out.strength & 3) << 2)
        | level
}

/// Inverse of [`pin_code`].
pub fn decode_pin_code(code: u32) -> Option<(u8, PinOut)> {
    let level = match code & 3 {
        0 => Some(false),
        1 => Some(true),
        2 => None,
        _ => return None,
    };
    (code >> 16 == 0 && code & 0xc0 == 0).then_some(())?;
    Some((
        (code >> 8) as u8,
        PinOut {
            level,
            strength: ((code >> 2) & 3) as u8,
            pull_up: code & (1 << 4) != 0,
            pull_down: code & (1 << 5) != 0,
        },
    ))
}

/// Every event code a pin can produce, for registering analog actions.
pub fn all_pin_codes(pin: u8) -> impl Iterator<Item = (u32, PinOut)> {
    (0..16u8).flat_map(move |bits| {
        [Some(false), Some(true), None].into_iter().map(move |level| {
            let out = PinOut {
                level,
                strength: bits & 3,
                pull_up: bits & 4 != 0,
                pull_down: bits & 8 != 0,
            };
            (pin_code(pin, out), out)
        })
    })
}

/// Event code telling the chip "the pad of `pin` is now at logic
/// `level`": bit 16 set, bits 15:8 pin, bit 0 level.
pub fn input_code(pin: u8, level: bool) -> u32 {
    (1 << 16) | (u32::from(pin) << 8) | u32::from(level)
}

/// Inverse of [`input_code`].
pub fn decode_input_code(code: u32) -> Option<(u8, bool)> {
    (code >> 16 == 1 && code & 0xfe == 0).then_some(((code >> 8) as u8, code & 1 != 0))
}

/// Event code asking for the voltage on ADC pad `pad` (GPIO0 to GPIO5):
/// bits 17:16 = 2, bits 15:8 pad.
pub fn adc_request_code(pad: u8) -> u32 {
    (2 << 16) | (u32::from(pad) << 8)
}

/// Inverse of [`adc_request_code`].
pub fn decode_adc_request(code: u32) -> Option<u8> {
    (code >> 16 == 2 && code & 0xff == 0).then_some((code >> 8) as u8)
}

/// The event codes that answer [`adc_request_code`] for `pad`: bit 31
/// set, bits 30:24 the pad, bits 23:0 the voltage in microvolts (0 to
/// 16.777 V; below 0 reads 0).
pub fn adc_value_code(pad: u8) -> ValueCode {
    ValueCode {
        base: (1 << 31) | (u32::from(pad & 0x7f) << 24),
        mask: 0x00ff_ffff,
        lsb: 1e-6,
    }
}

/// A threshold watch that turns a pad voltage into a logic level.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct InputWatch {
    /// Threshold (V).
    pub volts: f64,
    /// `true` for an upward crossing, `false` for a downward one.
    pub rising: bool,
    /// The [`input_code`] to send at the crossing.
    pub code: u32,
}

/// The two watches for an input pin with supply `vdd`: rising through
/// `V_IH` (0.75 VDD) reads high, falling through `V_IL` (0.25 VDD) reads
/// low. These are the datasheet's guaranteed limits; between them the
/// level holds, like a Schmitt trigger. (A real pad switches somewhere in
/// between, typically near half VDD.)
pub fn input_watches(pin: u8, vdd: f64) -> [InputWatch; 2] {
    [
        InputWatch {
            volts: VIH_FRACTION * vdd,
            rising: true,
            code: input_code(pin, true),
        },
        InputWatch {
            volts: VIL_FRACTION * vdd,
            rising: false,
            code: input_code(pin, false),
        },
    ]
}

/// The logic level of a pad at `volts` with nothing known before (the
/// start of a run): high at or above half VDD.
pub fn initial_input_level(volts: f64, vdd: f64) -> bool {
    volts >= 0.5 * vdd
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
        assert_eq!(seen.len(), 22 * 48);
        assert_eq!(decode_pin_code(3), None);
        assert_eq!(decode_pin_code(input_code(2, true)), None);
    }

    #[test]
    fn input_codes_round_trip_and_differ_from_pin_codes() {
        for pin in 0..22 {
            for level in [false, true] {
                let code = input_code(pin, level);
                assert_eq!(decode_input_code(code), Some((pin, level)));
                assert!(all_pin_codes(pin).all(|(c, _)| c != code));
            }
        }
        assert_eq!(decode_input_code(pin_code(3, PinOut::HIZ)), None);
        for pad in 0..6 {
            let req = adc_request_code(pad);
            assert_eq!(decode_adc_request(req), Some(pad));
            assert_eq!(decode_input_code(req), None);
            assert_eq!(decode_pin_code(req), None);
            let vc = adc_value_code(pad);
            let code = vc.encode(2.5);
            assert_eq!(vc.decode(code), Some(2.5));
            assert_eq!(decode_adc_request(code), None);
            assert_eq!(adc_value_code(pad ^ 1).decode(code), None);
        }
        let [hi, lo] = input_watches(4, 3.3);
        assert!((hi.volts - 2.475).abs() < 1e-12 && hi.rising && hi.code == input_code(4, true));
        assert!((lo.volts - 0.825).abs() < 1e-12 && !lo.rising && lo.code == input_code(4, false));
        assert!(initial_input_level(1.7, 3.3) && !initial_input_level(1.6, 3.3));
    }
}

/// The ESP32-C3 engine.
#[derive(Debug)]
pub struct Esp32c3Engine {
    name: String,
    /// The chip.
    pub soc: Esp32c3,
    links: Vec<PinLink>,
    /// ADC pads sampled from the analog engine: (pad, target).
    adc_links: Vec<(u8, u32)>,
    sent: usize,
    /// Pins whose level comes from [`input_code`] events.
    inputs: u32,
    /// Time claimed by a hart sleeping in `wfi` that has not idled there
    /// yet (so an input event can wake it earlier).
    idle_claim: Option<SimTime>,
    max_lead: Option<SimTime>,
}

impl Esp32c3Engine {
    /// Wrap a chip (firmware already loaded).
    pub fn new(name: &str, soc: Esp32c3) -> Self {
        Self {
            name: name.to_owned(),
            soc,
            links: Vec::new(),
            adc_links: Vec::new(),
            sent: 0,
            inputs: 0,
            idle_claim: None,
            max_lead: None,
        }
    }

    /// Forward changes of `link.pin` as events to `link.target`.
    pub fn link_pin(&mut self, link: PinLink) {
        self.links.push(link);
    }

    /// Sample ADC pad `pad` (GPIO0 to GPIO5) from engine `target`: each
    /// conversion on it sends [`adc_request_code`] there when it starts
    /// and uses the voltage in the [`adc_value_code`] answer.
    pub fn link_adc(&mut self, pad: u8, target: u32) {
        self.adc_links.retain(|&(p, _)| p != pad);
        self.adc_links.push((pad, target));
    }

    /// Take the pad level of `pin` from [`input_code`] events from now on,
    /// starting at `level` (for example [`initial_input_level`] of the
    /// circuit's operating point).
    pub fn link_input(&mut self, pin: u8, level: bool) {
        self.inputs |= 1 << pin;
        self.soc.bus.gpio.set_external(usize::from(pin), true);
        self.soc.set_pin_input(usize::from(pin), level);
    }

    /// Run at most `lead` ahead of the slice start while inputs are linked,
    /// bounding how late a busy hart can see an input interrupt (at the
    /// cost of more slices). `None` (the default) lets it run to the slice
    /// limit.
    pub fn set_max_lead(&mut self, lead: Option<SimTime>) {
        self.max_lead = lead.map(|l| SimTime(l.0.max(1)));
    }

    /// Let a lazily sleeping hart idle up to `t` (at most its claim).
    fn idle_to(&mut self, t: SimTime) {
        if let Some(claim) = self.idle_claim.take() {
            self.soc.hart.idle_until(t.min(claim));
        }
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
        let t = self.soc.hart.local_time();
        self.idle_claim.map_or(t, |c| c.max(t))
    }

    fn on_event(&mut self, ev: &Event, _cx: &mut SyncCtx<'_>) -> Result<(), EngineError> {
        if ev.code >> 31 == 1 {
            let pad = ((ev.code >> 24) & 0x7f) as u8;
            if let Some(volts) = adc_value_code(pad).decode(ev.code) {
                self.soc.bus.saradc.set_sample(pad, volts);
            }
            return Ok(());
        }
        if let Some((pin, level)) = decode_input_code(ev.code)
            && self.inputs >> pin & 1 != 0
        {
            self.idle_to(ev.at);
            self.soc.set_pin_input(usize::from(pin), level);
        }
        Ok(())
    }

    fn advance_to(&mut self, limit: SimTime, cx: &mut SyncCtx<'_>) -> Result<SimTime, EngineError> {
        self.idle_to(cx.now());
        let linked = self.inputs != 0;
        self.soc.bus.set_input_horizon(linked.then(|| cx.now()));
        let limit = match self.max_lead {
            Some(lead) if linked => limit.min(cx.now().saturating_add(lead)),
            _ => limit,
        };
        loop {
            let exit = self.soc.run_slice(limit);
            let edges = &self.soc.pin_edges()[self.sent..];
            self.sent = self.soc.pin_edges().len();
            // The earliest event sent: the engine returns there, so the
            // other engines see it at exactly that time.
            let mut first: Option<SimTime> = None;
            let mut sent = |cx: &mut SyncCtx<'_>, at: SimTime, target: u32, code: u32| {
                cx.schedule(at, target, code);
                first = Some(first.map_or(at, |f| f.min(at)));
            };
            for e in edges {
                for l in self.links.iter().filter(|l| l.pin == e.pin) {
                    sent(cx, e.at, l.target, pin_code(e.pin, e.out));
                }
            }
            for r in self.soc.bus.saradc.take_requests() {
                for &(_, target) in self.adc_links.iter().filter(|&&(p, _)| p == r.pad) {
                    sent(cx, r.at, target, adc_request_code(r.pad));
                }
            }
            if let Some(f) = first {
                return Ok(f.max(SimTime(cx.now().0 + 1)));
            }
            let t = self.soc.hart.local_time();
            match exit {
                RunExit::SyncRequested(_) if t < limit => {}
                RunExit::SyncRequested(_) | RunExit::BudgetExhausted => return Ok(t.max(limit)),
                // Only loads after the horizon (`cx.now()`) defer, so this
                // is progress.
                RunExit::Deferred(_) => return Ok(t),
                RunExit::WaitingForInterrupt if linked => {
                    self.idle_claim = Some(limit);
                    return Ok(limit);
                }
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
