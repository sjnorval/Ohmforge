//! E4b: `engine_blinky.rs` on `Session` (engine plan Task 12). The chip,
//! resistor, LED and PSU come from the model registry; `attach` does the pad
//! hooks; the assertions are the E3 ones.

use std::collections::BTreeMap;
use std::sync::Arc;

use of_analog::Edge;
use of_component_api::ParamValue;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::{HOST, SimTime, Stop};
use of_engine::{ComponentSpec, EngineBuilder, Firmware, Registry, SessionConfig};
use of_soc_esp32c3::Esp32c3Engine;

const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");
const R_SERIES: f64 = 330.0;
const LED_ON: u32 = 1;
const LED_OFF: u32 = 2;

fn registry() -> Registry {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
    let lib = Arc::new(Library::load_dir(&dir).expect("parts library"));
    let mut reg = Registry::new();
    register_builtin(&mut reg, lib, Corner::Typical, Env::default());
    of_soc_esp32c3::register(&mut reg);
    reg
}

/// One placement: ref, part id, model, params, (pin, net) list.
type Place<'a> = (
    &'a str,
    &'a str,
    &'a str,
    Vec<(&'a str, ParamValue)>,
    Vec<(&'a str, &'a str)>,
);

#[test]
fn session_blinky_matches_blinky_led() {
    let reg = registry();
    let mut eng = EngineBuilder::new("gnd");
    eng.net("vdd").net("gpio8").net("led_a");
    let places: Vec<Place<'_>> = vec![
        (
            "R1",
            "generic.resistor",
            "rust:resistor",
            vec![("resistance", R_SERIES.into())],
            vec![("1", "gpio8"), ("2", "led_a")],
        ),
        (
            "D1",
            "generic.led-red-0603",
            "rust:led",
            vec![],
            vec![("A", "led_a"), ("K", "gnd")],
        ),
        (
            "PSU",
            "bench.psu",
            "rust:bench-psu",
            // Near-ideal rail so LED current matches the Thevenin blinky_led fixture.
            vec![("series_r", 1e-4.into())],
            vec![("+", "vdd"), ("-", "gnd")],
        ),
        (
            "U1",
            "espressif.esp32-c3",
            "rust:esp32c3",
            vec![],
            vec![("3V3", "vdd"), ("GND", "gnd"), ("GPIO8", "gpio8")],
        ),
    ];
    for (refdes, part, model, params, pins) in places {
        let params: BTreeMap<String, ParamValue> = params.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        let names: Vec<String> = pins.iter().map(|(p, _)| (*p).to_owned()).collect();
        let firmware = (model == "rust:esp32c3").then(|| Firmware::Elf(BLINKY.to_vec()));
        let spec = ComponentSpec {
            refdes,
            part_id: part,
            params: &params,
            firmware,
            pins: &names,
            seed: 0,
        };
        let numeric = spec.numeric_params();
        let c = reg.build(model, spec).unwrap();
        let num: Vec<(&str, f64)> = numeric.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        eng.add(c, &pins, &num, 0).unwrap();
    }
    let mut s = eng.elaborate().unwrap().into_session(SessionConfig::default()).unwrap();
    s.analog().watch("v(led_a)", 1.0, Edge::Rising, HOST, LED_ON).unwrap();
    s.analog().watch("v(led_a)", 1.0, Edge::Falling, HOST, LED_OFF).unwrap();
    assert_eq!(
        s.engine_id("U1").map(|id| id.0),
        Some(0),
        "CPU engines are registered first"
    );

    let mut crossings = Vec::new();
    let mut on_current = Vec::new();
    let end = SimTime::from_us(2_600);
    loop {
        match s.run_until(end).unwrap() {
            Stop::Host(ev) => {
                crossings.push((ev.at, ev.code));
                if ev.code == LED_ON {
                    s.kernel().add_breakpoint(ev.at.saturating_add(SimTime::from_us(100)));
                }
            }
            Stop::Breakpoint(_) => {
                let i = (s.value("v(gpio8)").unwrap() - s.value("v(led_a)").unwrap()) / R_SERIES;
                on_current.push(i);
            }
            Stop::Reached(_) => break,
            other => panic!("unexpected stop {other:?}"),
        }
    }

    let soc = &s.engine_as::<Esp32c3Engine>("U1").unwrap().soc;
    let gpio: Vec<_> = soc
        .pin_edges()
        .iter()
        .filter(|e| e.pin == 8)
        .skip(1)
        .map(|e| e.at)
        .collect();
    assert_eq!(soc.uart_text(), "ohmforge blinky\n");
    assert_eq!(crossings.len(), gpio.len(), "crossings {crossings:?}");
    for (n, &(at, code)) in crossings.iter().enumerate() {
        assert_eq!(code, if n % 2 == 0 { LED_ON } else { LED_OFF });
        let lag = at.0.checked_sub(gpio[n].0).expect("crossing after the edge");
        assert!(lag < 1_000_000, "crossing {n} lags the GPIO edge by {lag} ps");
    }
    let on: Vec<_> = crossings.iter().filter(|c| c.1 == LED_ON).map(|c| c.0.0).collect();
    assert!(on.len() >= 5);
    for w in on.windows(2) {
        assert!(
            (w[1] - w[0]).abs_diff(500_000_000) < 1_000_000,
            "period {}",
            w[1] - w[0]
        );
    }
    assert!(!on_current.is_empty());
    for i in on_current {
        assert!((3e-3..4.5e-3).contains(&i), "LED on-current {i}");
    }
}
