//! The five demos built through the model registry and run on
//! [`of_engine::Session`], with the same sampling loops as the hand-wired
//! simulations. This is the engine half of trace parity (E4b): circuit files
//! (E4b-2, `parity::file`) build the same circuits and run here through [`run`].

use std::collections::BTreeMap;
use std::sync::Arc;

use of_component_api::ParamValue;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::{SimTime, Stop};
use of_engine::{ComponentSpec, EngineBuilder, Firmware, Registry, Session, SessionConfig};
use of_soc_esp32c3::Esp32c3Engine;

use super::{Demo, DemoRun, Raw, Step};
use crate::adc_pwm::{self, PwmKind};
use crate::button::{self, ButtonKind};
use crate::trace::{self, SampleClock, TraceBuf};

const BLINKY: &[u8] = include_bytes!("../../../../firmware/esp32c3-blinky/blinky.elf");
const R_SERIES: f64 = 330.0;

pub(crate) fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

/// The built-in registry over the repository's parts library.
pub(crate) fn registry() -> Registry {
    let lib = Arc::new(Library::load_dir(&parts_dir()).expect("parts library"));
    let mut reg = Registry::new();
    register_builtin(&mut reg, lib, Corner::Typical, Env::default());
    of_soc_esp32c3::register(&mut reg);
    reg
}

/// One placement: ref, part id, model, params, (pin, net) list, firmware.
struct Place {
    refdes: &'static str,
    part: &'static str,
    model: &'static str,
    params: Vec<(&'static str, ParamValue)>,
    pins: Vec<(&'static str, &'static str)>,
    firmware: Option<Firmware>,
}

fn place(
    refdes: &'static str,
    part: &'static str,
    model: &'static str,
    params: Vec<(&'static str, ParamValue)>,
    pins: Vec<(&'static str, &'static str)>,
) -> Place {
    Place {
        refdes,
        part,
        model,
        params,
        pins,
        firmware: None,
    }
}

fn session(places: Vec<Place>, max_step: f64) -> Result<Session, String> {
    let reg = registry();
    let mut eng = EngineBuilder::new("gnd");
    for p in &places {
        for (_, net) in &p.pins {
            eng.net(net);
        }
    }
    for p in places {
        let params: BTreeMap<String, ParamValue> = p.params.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        let pins: Vec<String> = p.pins.iter().map(|(pin, _)| (*pin).to_owned()).collect();
        let spec = ComponentSpec {
            refdes: p.refdes,
            part_id: p.part,
            params: &params,
            firmware: p.firmware,
            pins: &pins,
            seed: 0,
        };
        let numeric = spec.numeric_params();
        let c = reg.build(p.model, spec).map_err(|e| e.to_string())?;
        let num: Vec<(&str, f64)> = numeric.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        eng.add(c, &p.pins, &num, 0).map_err(|e| e.to_string())?;
    }
    eng.elaborate()
        .map_err(|e| e.to_string())?
        .into_session(SessionConfig {
            quantum: SimTime::from_us(50),
            max_step,
        })
        .map_err(|e| e.to_string())
}

fn led_and_psu(r_ref: &'static str) -> Vec<Place> {
    vec![
        place(
            r_ref,
            "generic.resistor",
            "rust:resistor",
            vec![("resistance", R_SERIES.into())],
            vec![("1", "gpio8"), ("2", "led_a")],
        ),
        place(
            "D1",
            "generic.led-red-0603",
            "rust:led",
            vec![],
            vec![("A", "led_a"), ("K", "gnd")],
        ),
        place(
            "PSU",
            "bench.psu",
            "rust:bench-psu",
            vec![],
            vec![("+", "vdd"), ("-", "gnd")],
        ),
    ]
}

fn chip(firmware: Firmware, gpios: &[&'static str], inputs: &[&str]) -> Place {
    let mut pins = vec![("3V3", "vdd"), ("GND", "gnd")];
    for &g in gpios {
        let net = match g {
            "GPIO2" => "gpio2",
            "GPIO8" => "gpio8",
            "GPIO9" => "gpio9",
            other => panic!("no net for {other}"),
        };
        pins.push((g, net));
    }
    let mut params = vec![("quiescent_a", ParamValue::Number(0.0))];
    if !inputs.is_empty() {
        params.push((
            "inputs",
            ParamValue::List(inputs.iter().map(|s| (*s).to_owned()).collect()),
        ));
    }
    Place {
        refdes: "U1",
        part: "espressif.esp32-c3",
        model: "rust:esp32c3",
        params,
        pins,
        firmware: Some(firmware),
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Loop {
    Blink,
    Pwm,
    Button,
}

/// A demo on [`Session`] with the legacy sampling loop.
pub(crate) struct SessionDemo {
    s: Session,
    style: Loop,
    samples: TraceBuf,
    psu_samples: TraceBuf,
    clock: SampleClock,
    /// Button loop: next grid sample (relative).
    next_sample: SimTime,
    closed: bool,
}

/// Build `demo` on a `Session` (parity runs: U1 `quiescent_a = 0`).
pub(crate) fn start(demo: Demo) -> Result<Box<dyn DemoRun>, String> {
    let (places, max_step, style) = match demo {
        Demo::Blink => {
            let mut p = led_and_psu("R1");
            p.push(chip(Firmware::Elf(BLINKY.to_vec()), &["GPIO8"], &[]));
            (p, 5e-6, Loop::Blink)
        }
        Demo::Dimmer | Demo::Fade => {
            let kind = if demo == Demo::Dimmer {
                PwmKind::Dimmer
            } else {
                PwmKind::Fade
            };
            let soc = adc_pwm::booted(kind)?;
            let mut p = Vec::new();
            let mut gpios = vec!["GPIO8"];
            if demo == Demo::Dimmer {
                p.push(place(
                    "RV1",
                    "bourns.ptv09a-b103",
                    "rust:potentiometer",
                    vec![("position", 0.5.into())],
                    vec![("1", "gnd"), ("W", "gpio2"), ("3", "vdd")],
                ));
                gpios.insert(0, "GPIO2");
            }
            p.extend(led_and_psu("R2"));
            p.push(chip(Firmware::Booted(Box::new(soc)), &gpios, &[]));
            (p, 20e-6, Loop::Pwm)
        }
        Demo::Button | Demo::ButtonIrq => {
            let kind = if demo == Demo::Button {
                ButtonKind::Poll
            } else {
                ButtonKind::Irq
            };
            let soc = button::booted(kind)?;
            let mut p = vec![place(
                "SW1",
                "generic.pushbutton",
                "rust:switch",
                vec![],
                vec![("1", "gpio9"), ("2", "gnd")],
            )];
            p.extend(led_and_psu("R2"));
            p.push(chip(Firmware::Booted(Box::new(soc)), &["GPIO8", "GPIO9"], &["GPIO9"]));
            (p, 5e-6, Loop::Button)
        }
    };
    let s = session(places, max_step)?;
    wrap(s, style)
}

/// The legacy sampling loop of `demo`.
fn style(demo: Demo) -> Loop {
    match demo {
        Demo::Blink => Loop::Blink,
        Demo::Dimmer | Demo::Fade => Loop::Pwm,
        Demo::Button | Demo::ButtonIrq => Loop::Button,
    }
}

/// Run an already built `Session` of `demo` (for example one built from its
/// circuit file) with the legacy sampling loop.
pub(crate) fn run(s: Session, demo: Demo) -> Result<Box<dyn DemoRun>, String> {
    wrap(s, style(demo))
}

fn wrap(s: Session, style: Loop) -> Result<Box<dyn DemoRun>, String> {
    let mut d = SessionDemo {
        s,
        style,
        samples: TraceBuf::default_scope(),
        psu_samples: TraceBuf::default_scope(),
        clock: SampleClock::new(0.0, trace::SAMPLE_STEP),
        next_sample: SimTime::ZERO,
        closed: false,
    };
    d.cpu().start_edge_log();
    d.record_sample()?;
    if style == Loop::Button {
        d.clock.take(d.time());
        d.next_sample = SimTime::from_secs_f64(d.clock.next().max(0.0));
    }
    Ok(Box::new(d))
}

fn reached(stop: Stop, at: f64) -> Result<(), String> {
    match stop {
        Stop::Reached(_) => Ok(()),
        other => Err(format!("ESP32-C3 stopped at {:.3} ms ({other:?})", at * 1e3)),
    }
}

impl SessionDemo {
    fn cpu(&mut self) -> &mut Esp32c3Engine {
        self.s.engine_as::<Esp32c3Engine>("U1").expect("U1 engine")
    }

    fn time(&self) -> f64 {
        self.s.now().as_secs_f64()
    }

    fn run_to(&mut self, t: SimTime) -> Result<(), String> {
        let stop = self.s.run_until(t).map_err(|e| e.to_string())?;
        reached(stop, self.time())
    }

    fn record_sample(&mut self) -> Result<(), String> {
        let t = self.time();
        let gpio = self.s.value("v(gpio8)").ok_or("no v(gpio8)")?;
        let anode = self.s.value("v(led_a)").ok_or("no v(led_a)")?;
        self.samples.push(t, (gpio - anode) * 1e3 / R_SERIES);
        self.cpu().recycle_consumed_edges();
        self.s.commit().map_err(|e| e.to_string())?;
        let amps = self
            .s
            .readings("PSU")
            .into_iter()
            .find(|(k, _)| k == "regulated_amps")
            .map(|(_, v)| v)
            .ok_or("no PSU reading")?;
        self.psu_samples.push(t, amps * 1e3);
        Ok(())
    }

    fn advance(&mut self, seconds: f64) -> Result<(), String> {
        match self.style {
            Loop::Blink => self.advance_blink(seconds),
            Loop::Pwm => self.advance_pwm(seconds),
            Loop::Button => self.advance_button(seconds),
        }
    }

    /// `Blink::advance`.
    fn advance_blink(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        while self.s.now() < until {
            let grid = SimTime::from_secs_f64(self.clock.next().max(0.0));
            let next = grid.min(until);
            if next > self.s.now() {
                self.run_to(next)?;
            } else if self.s.now() < until {
                self.run_to(until)?;
            }
            if self.clock.take(self.time()) {
                self.record_sample()?;
            }
        }
        Ok(())
    }

    /// `PwmSim::advance`: samples on the grid and at LEDC events.
    fn advance_pwm(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        let origin = self.s.origin();
        while self.s.now() < until {
            let now = self.s.now();
            let grid = SimTime::from_secs_f64(self.clock.next().max(0.0));
            let pwm = self
                .cpu()
                .soc
                .bus
                .ledc
                .next_event()
                .map(|t| SimTime(t.0.saturating_sub(origin.0)));
            if grid <= now {
                if self.clock.take(self.time()) {
                    self.record_sample()?;
                } else if now < until {
                    let nudge = now.saturating_add(SimTime::from_ns(1)).min(until);
                    self.run_to(nudge)?;
                }
                continue;
            }
            let mut next = grid.min(until);
            if let Some(p) = pwm
                && p > now
                && p < next
            {
                next = p;
            }
            if next > self.s.now() {
                self.run_to(next)?;
            } else if self.s.now() < until {
                self.run_to(until)?;
            }
            let hit_grid = self.clock.take(self.time());
            let hit_pwm = pwm.is_some_and(|p| p <= self.s.now());
            if hit_grid || hit_pwm {
                self.record_sample()?;
            }
        }
        Ok(())
    }

    /// `ButtonSim::advance` (bounce timers are the switch's, inside `Session`).
    fn advance_button(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        while self.s.now() < until {
            let next = self.next_sample.min(until);
            if next > self.s.now() {
                self.run_to(next)?;
            }
            if self.s.now() >= self.next_sample {
                if self.clock.take(self.time()) {
                    self.record_sample()?;
                }
                self.next_sample = SimTime::from_secs_f64(self.clock.next().max(0.0));
                if self.next_sample <= self.s.now() {
                    self.next_sample = self.s.now().saturating_add(SimTime::from_ns(1));
                }
            }
        }
        Ok(())
    }
}

impl DemoRun for SessionDemo {
    fn step(&mut self, step: Step) -> Result<(), String> {
        match step {
            Step::RunTo(t) => self.advance(t),
            Step::Position(p) => {
                self.s.set_param("RV1", "position", p).map_err(|e| e.to_string())?;
                // As PwmSim::set_position: one short step so the wiper updates.
                self.advance(self.time() + 1e-6)
            }
            Step::Press(closed) => {
                let idle = self.s.pending_timers().next().is_none();
                if self.closed == closed && idle {
                    return Ok(());
                }
                self.closed = closed;
                self.s
                    .set_param("SW1", "closed", if closed { 1.0 } else { 0.0 })
                    .map_err(|e| e.to_string())?;
                self.advance(self.time() + 1e-6)
            }
            Step::Bounce(on) => self
                .s
                .set_param("SW1", "bounce", if on { 1.0 } else { 0.0 })
                .map_err(|e| e.to_string()),
        }
    }

    fn raw(&mut self) -> Raw {
        let origin = self.s.origin();
        let led = self.samples.to_vec();
        let psu = self.psu_samples.to_vec();
        let cpu = self.cpu();
        Raw {
            edges: cpu.logged_edges(),
            origin,
            uart: cpu.soc.uart_text(),
            led,
            psu,
        }
    }
}
