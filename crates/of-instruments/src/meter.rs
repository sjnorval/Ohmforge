//! The ideal multimeter (engine E5c, RFC 0004): a voltmeter between two nets
//! or an ammeter through one part, on any circuit.
//!
//! - Placement: [`candidates`] lists the parts with a pin on each of the two
//!   clicked nets; with more than one the user picks (the desktop chooser).
//! - Ammeter direction is from net `a` toward net `b` through the part. For a
//!   two-pin part that is the current into its pin on `a`. For a part with
//!   more pins (a potentiometer's `1`–`W` or `W`–`3`) it is read at whichever
//!   of the two pins is an end pin of the part (first or last in pin order):
//!   into the pin on `a`, or out of the pin on `b`.
//! - Readings average over whole PWM periods of a reference channel, as a
//!   real DMM's integrating ADC does ([`Averaging`]; `stats::average_periods`).
//! - The ground net is `0` or `gnd` (the schematic uses `0`, circuit files `gnd`).

use crate::profile::{AmpInput, Display, DmmProfile};
use crate::service::{ChannelId, MeasurementService};
use crate::signal::Signal;
use crate::stats::average_periods;

/// A part's pins and their nets, in the part's pin order (`terminal_currents` order).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PartPins {
    /// Reference designator.
    pub refdes: String,
    /// `(pin, net)` in pin order.
    pub pins: Vec<(String, String)>,
}

/// One part an ammeter between two nets could go through.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Candidate {
    /// Reference designator.
    pub refdes: String,
    /// What the ammeter samples.
    pub signal: Signal,
    /// `true` when the signal is the current out of the pin on `b`
    /// (the reading is its negative).
    pub negate: bool,
}

/// What the multimeter measures.
#[derive(Debug, Clone, PartialEq)]
pub enum MeterMode {
    /// `V(a) − V(b)`.
    DcVolts,
    /// Current from `a` toward `b` through one part.
    DcAmps,
}

/// Why a meter cannot be placed or read.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MeterError {
    /// Both probes are on the same net.
    SameNet,
    /// No part has a pin on both nets.
    NoUniqueBranch,
}

impl MeterError {
    /// The message the desktop shows.
    pub fn message(&self) -> &'static str {
        match self {
            Self::SameNet => "those points are on the same net",
            Self::NoUniqueBranch => "no single branch joins those points",
        }
    }
}

/// How readings are averaged: over `periods` whole periods of `reference`
/// (rising edges above `high`), or over `fallback` seconds when no period is seen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Averaging {
    /// Channel whose rising edges mark the periods (the LED current today).
    pub reference: ChannelId,
    /// Threshold for a rising edge, in the reference channel's unit.
    pub high: f64,
    /// Whole periods averaged.
    pub periods: usize,
    /// Window (s) when no period is visible.
    pub fallback: f64,
}

/// A placed ideal multimeter.
#[derive(Debug, Clone, PartialEq)]
pub struct Multimeter {
    /// Voltmeter or ammeter.
    pub mode: MeterMode,
    /// What it samples (a [`Signal::DiffVolts`] or a pin current).
    pub signal: Signal,
    /// The ammeter reading is the negative of the signal.
    pub negate: bool,
    /// The part an ammeter goes through.
    pub part: Option<String>,
}

/// A meter's live value.
#[derive(Debug, Clone, PartialEq)]
pub enum Reading {
    /// Volts.
    Volts(f64),
    /// Amps through `part`.
    Amps {
        /// Current (A), from `a` toward `b`.
        amps: f64,
        /// The part.
        part: String,
    },
    /// Not measurable.
    Error(MeterError),
    /// No samples yet.
    Missing,
}

impl Reading {
    /// The text on the schematic and in the meter list (unchanged from the
    /// pre-E5 meters: `1.234 V`, `12.34 mA`, `123 mA`).
    pub fn display(&self) -> String {
        match self {
            Self::Volts(v) => format!("{v:.3} V"),
            Self::Amps { amps, .. } => {
                let ma = amps * 1e3;
                if ma.abs() >= 100.0 {
                    format!("{ma:.0} mA")
                } else {
                    format!("{ma:.2} mA")
                }
            }
            Self::Error(e) => e.message().to_owned(),
            Self::Missing => "—".into(),
        }
    }

    /// What a real-world multimeter shows for this reading: the profile's
    /// autoranged value and ± band (`None` for an error or no samples yet).
    /// Amps use the current `input` jack.
    pub fn on(&self, profile: &DmmProfile, input: AmpInput) -> Option<Display> {
        match self {
            Self::Volts(v) => Some(profile.dc_volts(*v)),
            Self::Amps { amps, .. } => Some(profile.dc_amps(input, *amps)),
            Self::Error(_) | Self::Missing => None,
        }
    }
}

fn is_ground(net: &str) -> bool {
    net == "0" || net.eq_ignore_ascii_case("gnd")
}

/// Same net, with `0` and `gnd` both naming ground.
pub fn same_net(a: &str, b: &str) -> bool {
    a.eq_ignore_ascii_case(b) || (is_ground(a) && is_ground(b))
}

/// The parts an ammeter from net `a` to net `b` could go through, in part order.
pub fn candidates(parts: &[PartPins], a: &str, b: &str) -> Result<Vec<Candidate>, MeterError> {
    if same_net(a, b) {
        return Err(MeterError::SameNet);
    }
    let mut out = Vec::new();
    for p in parts {
        let on = |net: &str| p.pins.iter().position(|(_, n)| same_net(n, net));
        let (Some(ia), Some(ib)) = (on(a), on(b)) else {
            continue;
        };
        let end = |i: usize| i == 0 || i + 1 == p.pins.len();
        let (pin, negate) = if end(ia) {
            (ia, false)
        } else if end(ib) {
            (ib, true)
        } else {
            continue;
        };
        out.push(Candidate {
            refdes: p.refdes.clone(),
            signal: Signal::PinCurrent {
                refdes: p.refdes.clone(),
                pin: Some(p.pins[pin].0.clone()),
            },
            negate,
        });
    }
    if out.is_empty() {
        Err(MeterError::NoUniqueBranch)
    } else {
        Ok(out)
    }
}

impl Multimeter {
    /// A voltmeter reading `V(a) − V(b)`.
    pub fn voltmeter(a: &str, b: &str) -> Self {
        Self {
            mode: MeterMode::DcVolts,
            signal: Signal::DiffVolts(a.to_owned(), b.to_owned()),
            negate: false,
            part: None,
        }
    }

    /// An ammeter through `candidate`.
    pub fn ammeter(candidate: &Candidate) -> Self {
        Self {
            mode: MeterMode::DcAmps,
            signal: candidate.signal.clone(),
            negate: candidate.negate,
            part: Some(candidate.refdes.clone()),
        }
    }

    /// The reading now, averaged as `avg` says over the samples the service
    /// holds for [`Self::signal`] (watch it first).
    pub fn read(&self, svc: &MeasurementService, avg: &Averaging, now: f64) -> Reading {
        let ring = |sig: &Signal| {
            svc.channels()
                .find(|(_, c)| c.signal == *sig)
                .map(|(_, c)| c.ring.to_vec())
        };
        let Some(values) = ring(&self.signal).filter(|v| !v.is_empty()) else {
            return Reading::Missing;
        };
        let reference = svc.channel(avg.reference).map(|c| c.ring.to_vec()).unwrap_or_default();
        let mean = average_periods(&values, &reference, now, avg.high, avg.periods, avg.fallback);
        match self.mode {
            MeterMode::DcVolts => Reading::Volts(mean),
            MeterMode::DcAmps => Reading::Amps {
                amps: if self.negate { -mean } else { mean },
                part: self.part.clone().unwrap_or_default(),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_reading_shows_on_a_real_meter() {
        let fluke = crate::profile::multimeters().find(|p| p.id == "fluke.87v").unwrap();
        let v = Reading::Volts(1.6492).on(fluke, AmpInput::Milli).unwrap();
        assert_eq!(v.to_string(), "1.649 ± 0.002 V");
        let a = Reading::Amps {
            amps: 22.98e-3,
            part: "R1".into(),
        };
        assert_eq!(a.on(fluke, AmpInput::Milli).unwrap().to_string(), "22.98 ± 0.09 mA");
        assert!(Reading::Missing.on(fluke, AmpInput::Milli).is_none());
    }
    use crate::service::{ProbeError, Probeable, SamplePlan};
    use std::collections::BTreeMap;

    fn part(refdes: &str, pins: &[(&str, &str)]) -> PartPins {
        PartPins {
            refdes: refdes.into(),
            pins: pins.iter().map(|(p, n)| ((*p).into(), (*n).into())).collect(),
        }
    }

    fn dimmer() -> Vec<PartPins> {
        vec![
            part(
                "U1",
                &[("3V3", "vdd"), ("GPIO2", "gpio2"), ("GPIO8", "gpio8"), ("GND", "gnd")],
            ),
            part("R2", &[("1", "gpio8"), ("2", "led_a")]),
            part("D1", &[("A", "led_a"), ("K", "gnd")]),
            part("RV1", &[("1", "gnd"), ("W", "gpio2"), ("3", "vdd")]),
            part("PSU", &[("+", "vdd"), ("-", "gnd")]),
            part("GND", &[("1", "gnd")]),
        ]
    }

    fn names(c: &[Candidate]) -> Vec<&str> {
        c.iter().map(|c| c.refdes.as_str()).collect()
    }

    #[test]
    fn a_two_pin_part_is_read_into_its_pin_on_a() {
        let c = candidates(&dimmer(), "gpio8", "led_a").unwrap();
        assert_eq!(names(&c), ["R2"]);
        assert_eq!(c[0].signal, "i(R2.1)".parse().unwrap());
        assert!(!c[0].negate);
        let c = candidates(&dimmer(), "led_a", "gpio8").unwrap();
        assert_eq!(c[0].signal, "i(R2.2)".parse().unwrap());
        // Ground is `0` on the schematic and `gnd` in the file.
        let c = candidates(&dimmer(), "led_a", "0").unwrap();
        assert_eq!(names(&c), ["D1"]);
        assert_eq!(c[0].signal, "i(D1.A)".parse().unwrap());
    }

    #[test]
    fn a_pot_segment_is_read_at_its_end_pin() {
        // 1–W: into pin 1 (an end pin, on a).
        // (U1 also has a pin on both nets, so it is offered too.)
        let c = candidates(&dimmer(), "0", "gpio2").unwrap();
        assert_eq!(names(&c), ["U1", "RV1"]);
        assert_eq!((c[1].signal.to_string(), c[1].negate), ("i(RV1.1)".into(), false));
        // W–3: out of pin 3 (the end pin is on b).
        let c = candidates(&dimmer(), "gpio2", "vdd").unwrap();
        assert_eq!(names(&c), ["U1", "RV1"]);
        assert_eq!((c[1].signal.to_string(), c[1].negate), ("i(RV1.3)".into(), true));
    }

    #[test]
    fn parallel_parts_are_all_offered_and_errors_are_kept() {
        let c = candidates(&dimmer(), "vdd", "0").unwrap();
        assert_eq!(names(&c), ["U1", "RV1", "PSU"]);
        let psu = &c[2];
        assert_eq!((psu.signal.to_string(), psu.negate), ("i(PSU.+)".into(), false));
        assert_eq!(candidates(&dimmer(), "gnd", "0"), Err(MeterError::SameNet));
        assert_eq!(candidates(&dimmer(), "led_a", "vdd"), Err(MeterError::NoUniqueBranch));
        assert_eq!(
            MeterError::NoUniqueBranch.message(),
            "no single branch joins those points"
        );
    }

    #[test]
    fn display_is_unchanged_from_the_old_meters() {
        assert_eq!(Reading::Volts(1.65).display(), "1.650 V");
        let a = |amps| {
            Reading::Amps {
                amps,
                part: "R1".into(),
            }
            .display()
        };
        assert_eq!(a(3.9485e-3), "3.95 mA");
        assert_eq!(a(-0.0229839), "-22.98 mA");
        assert_eq!(a(0.1234), "123 mA");
        assert_eq!(Reading::Missing.display(), "—");
    }

    struct Fake {
        t: f64,
        v: BTreeMap<Signal, f64>,
    }

    impl Probeable for Fake {
        fn now_secs(&self) -> f64 {
            self.t
        }
        fn sample(&mut self, s: &Signal) -> Result<f64, ProbeError> {
            self.v.get(s).copied().ok_or_else(|| ProbeError::Unresolved(s.clone()))
        }
    }

    #[test]
    fn readings_average_whole_pwm_periods() {
        // 1 kHz, 25 % duty: reference 4 mA high; the meter's current 2 mA high.
        let mut svc = MeasurementService::new(SamplePlan::Every(1e-3));
        let led: Signal = "i(R2)".parse().unwrap();
        let reference = svc.watch_default(led.clone());
        let c = candidates(&dimmer(), "vdd", "0").unwrap();
        let am = Multimeter::ammeter(&c[2]);
        assert_eq!(am.mode, MeterMode::DcAmps);
        assert_eq!(am.part.as_deref(), Some("PSU"));
        svc.watch_default(am.signal.clone());
        let vm = Multimeter::voltmeter("vdd", "0");
        assert_eq!(vm.signal, "v(vdd,0)".parse().unwrap());
        svc.watch_default(vm.signal.clone());
        let mut src = Fake {
            t: 0.0,
            v: BTreeMap::new(),
        };
        for k in 0..=40 {
            let t0 = k as f64 * 1e-3;
            let edges: &[(f64, bool)] = if k < 40 {
                &[(t0, true), (t0 + 0.25e-3, false)]
            } else {
                &[(t0, true)]
            };
            for &(t, on) in edges {
                src.t = t;
                src.v.insert(led.clone(), if on { 4e-3 } else { 0.0 });
                src.v.insert(am.signal.clone(), if on { 2e-3 } else { 0.0 });
                src.v.insert(vm.signal.clone(), if on { 3.0 } else { 3.2 });
                svc.record(&mut src).unwrap();
            }
        }
        let avg = Averaging {
            reference,
            high: 0.5e-3,
            periods: 2,
            fallback: 2e-3,
        };
        let now = 40e-3;
        match am.read(&svc, &avg, now) {
            Reading::Amps { amps, part } => {
                assert!((amps - 0.5e-3).abs() < 1e-15, "{amps}");
                assert_eq!(part, "PSU");
            }
            r => panic!("{r:?}"),
        }
        match vm.read(&svc, &avg, now) {
            Reading::Volts(v) => assert!((v - 3.15).abs() < 1e-12, "{v}"),
            r => panic!("{r:?}"),
        }
        let unwatched = Multimeter::voltmeter("gpio8", "0");
        assert_eq!(unwatched.read(&svc, &avg, now), Reading::Missing);
    }
}
