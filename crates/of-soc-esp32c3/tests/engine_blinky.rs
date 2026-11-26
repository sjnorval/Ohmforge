//! E3 acceptance: an engine-built blinky matches `blinky_led` (edge times and
//! LED on-current within the existing tolerances).

use std::collections::BTreeMap;
use std::sync::Arc;

use of_analog::{AnalogEngine, Edge, EngineConfig, Options};
use of_components::analog::{Env, Library};
use of_components::{BenchPsu, LibraryComponent, SupplySettings};
use of_core::{HOST, Kernel, SimTime, Stop};
use of_engine::EngineBuilder;
use of_parts::Corner;
use of_soc_esp32c3::{Esp32c3Component, Esp32c3Engine, hook_pin};

const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");
const R_SERIES: f64 = 330.0;
const LED_ON: u32 = 1;
const LED_OFF: u32 = 2;

fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

#[test]
fn engine_blinky_matches_blinky_led() {
    let lib = Arc::new(Library::load_dir(&parts_dir()).expect("parts library"));

    let mut chip = Esp32c3Component::with_elf("U1", BLINKY, &["3V3", "GND", "GPIO8"]).expect("chip pins");
    chip.set_analog_target(1);

    let mut eng = EngineBuilder::new("gnd");
    eng.net("vdd").net("gpio8").net("led_a");

    let r1 = LibraryComponent::new(
        "R1",
        "generic.resistor",
        Arc::clone(&lib),
        BTreeMap::from([("resistance".into(), R_SERIES)]),
        Corner::Typical,
        Env::default(),
    )
    .unwrap();
    let d1 = LibraryComponent::new(
        "D1",
        "generic.led-red-0603",
        Arc::clone(&lib),
        BTreeMap::new(),
        Corner::Typical,
        Env::default(),
    )
    .unwrap();
    let psu = BenchPsu::new(
        "PSU1",
        SupplySettings {
            on: true,
            volts: 3.3,
            // Near-ideal rail so LED current matches the Thevenin blinky_led fixture.
            series_r: 1e-4,
            current_limit: 0.5,
            rated: None,
        },
    );

    eng.add(
        Box::new(r1),
        &[("1", "gpio8"), ("2", "led_a")],
        &[("resistance", R_SERIES)],
        0,
    )
    .unwrap();
    eng.add(Box::new(d1), &[("A", "led_a"), ("K", "gnd")], &[], 0).unwrap();
    eng.add(
        Box::new(psu),
        &[("+", "vdd"), ("-", "gnd")],
        &[("volts", 3.3), ("series_r", 1e-4)],
        0,
    )
    .unwrap();
    eng.add(
        Box::new(chip),
        &[("3V3", "vdd"), ("GND", "gnd"), ("GPIO8", "gpio8")],
        &[],
        0,
    )
    .unwrap();

    let mut built = eng.elaborate().expect("elaborate");
    let circuit = std::mem::replace(&mut built.circuit, of_analog::Circuit::new());
    let mut analog = AnalogEngine::new(
        "analog",
        circuit,
        Options::default(),
        EngineConfig {
            max_step: 5e-6,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    hook_pin(&mut analog, "U1", 8);
    analog.watch("v(led_a)", 1.0, Edge::Rising, HOST, LED_ON).unwrap();
    analog.watch("v(led_a)", 1.0, Edge::Falling, HOST, LED_OFF).unwrap();

    let chip_i = built
        .components
        .iter()
        .position(|c| c.component.info().part_id == "espressif.esp32-c3")
        .expect("U1");
    let cpu = built.components[chip_i].component.take_engine().expect("Esp32c3Engine");

    let mut k = Kernel::new(SimTime::from_us(50));
    let cpu = k.add_engine(cpu);
    let an = k.add_engine(Box::new(analog));
    assert_eq!(cpu.0, 0);
    assert_eq!(an.0, 1);

    let mut crossings = Vec::new();
    let mut on_current = Vec::new();
    let end = SimTime::from_us(2_600);
    loop {
        match k.run_until(end).unwrap() {
            Stop::Host(ev) => {
                crossings.push((ev.at, ev.code));
                if ev.code == LED_ON {
                    k.add_breakpoint(ev.at.saturating_add(SimTime::from_us(100)));
                }
            }
            Stop::Breakpoint(_) => {
                let a = k.engine_as::<AnalogEngine>(an).unwrap();
                let i = (a.value("v(gpio8)").unwrap() - a.value("v(led_a)").unwrap()) / R_SERIES;
                on_current.push(i);
            }
            Stop::Reached(_) => break,
            other => panic!("unexpected stop {other:?}"),
        }
    }

    let soc = &k.engine_as::<Esp32c3Engine>(cpu).unwrap().soc;
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
            "period {} ps",
            w[1] - w[0]
        );
    }
    assert!(!on_current.is_empty());
    for i in on_current {
        assert!((3.0e-3..4.5e-3).contains(&i), "LED on-current {i} A");
    }
}
