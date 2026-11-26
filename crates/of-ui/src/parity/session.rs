//! The five demos built through the model registry and run on
//! [`of_engine::Session`], with the desktop's sampling loops. This is the engine half of trace parity (E4b): circuit files
//! (E4b-2, `parity::file`) build the same circuits and run here through [`run`].
//! Both run on the desktop's [`SessionSim`] (E4c).

use std::collections::BTreeMap;
use std::sync::Arc;

use of_component_api::ParamValue;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::SimTime;
use of_engine::{ComponentSpec, EngineBuilder, Firmware, Registry, Session, SessionConfig};

use super::{Demo, DemoRun, Raw, Step};
use crate::adc_pwm::{self, PwmKind};
use crate::button::{self, ButtonKind};
use crate::session_sim::SessionSim;

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
            ..SessionConfig::default()
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
            // The ESP32-C3's vdd rating, as the circuit builder derives it
            // from the parts on the + rail.
            vec![("rated_min", 3.0.into()), ("rated_max", 3.6.into())],
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

/// A demo on [`Session`] with the parity sampling loop: the desktop's own
/// [`SessionSim`], so parity checks the code the desktop runs.
pub(crate) struct SessionDemo(SessionSim);

/// Build `demo` on a `Session` (parity runs: U1 `quiescent_a = 0`).
pub(crate) fn start(demo: Demo) -> Result<Box<dyn DemoRun>, String> {
    run(build(demo)?, demo)
}

/// Place `demo` through the model registry in Rust (U1 `quiescent_a = 0`),
/// independently of its circuit file.
pub(crate) fn build(demo: Demo) -> Result<Session, String> {
    let (places, max_step) = match demo {
        Demo::Blink => {
            let mut p = led_and_psu("R1");
            p.push(chip(Firmware::Elf(BLINKY.to_vec()), &["GPIO8"], &[]));
            (p, 5e-6)
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
            (p, 20e-6)
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
            (p, 5e-6)
        }
    };
    session(places, max_step)
}

/// Run an already built `Session` of `demo` (for example one built from its
/// circuit file) with the parity sampling loop.
pub(crate) fn run(s: Session, demo: Demo) -> Result<Box<dyn DemoRun>, String> {
    Ok(Box::new(SessionDemo(SessionSim::run_logged(demo.kind(), s)?)))
}

impl DemoRun for SessionDemo {
    fn step(&mut self, step: Step) -> Result<(), String> {
        let sim = &mut self.0;
        match step {
            Step::RunTo(t) => sim.advance(t),
            Step::Position(p) => sim.set_position(p),
            Step::Press(closed) => sim.set_closed(closed),
            Step::Bounce(on) => sim
                .session()
                .set_param("SW1", "bounce", if on { 1.0 } else { 0.0 })
                .map_err(|e| e.to_string()),
        }
    }

    fn raw(&mut self) -> Raw {
        self.0.parity_raw()
    }
}
