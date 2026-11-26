//! End to end: the blinky firmware on the ESP32-C3 engine drives a
//! parts-library LED and resistor in the analog engine through the kernel.
//! The LED current must switch with the firmware's 500 µs period.

use of_analog::devices::PinDriver;
use of_analog::{AnalogEngine, Edge, EngineConfig, Options, SetParam};
use of_components::analog::{Board, Library, Placement, build};
use of_core::{HOST, Kernel, SimTime, Stop};
use of_firmware::ElfImage;
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, PinLink, all_pin_codes};

const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");
const VDD: f64 = 3.3;
const R_SERIES: f64 = 330.0;
const LED_ON: u32 = 1;
const LED_OFF: u32 = 2;

#[test]
fn led_current_follows_the_firmware() {
    let lib = Library::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")).unwrap();
    let board = Board {
        placements: vec![
            Placement::new("R1", "generic.resistor", &[("1", "gpio8"), ("2", "led_a")]).with("resistance", R_SERIES),
            Placement::new("D1", "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
        ],
        ..Board::default()
    };
    let mut b = build(&lib, &board).unwrap();
    let pin = b.circuit.node("gpio8");
    b.circuit.add(PinDriver::new("u1_gpio8", pin, 0, 0.0, 1e9)).unwrap();

    let mut analog = AnalogEngine::new(
        "analog",
        b.circuit,
        Options::default(),
        EngineConfig {
            max_step: 5e-6,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    for (code, out) in all_pin_codes(8) {
        let (v, r) = out.thevenin(VDD);
        let set = |param: &str, value| SetParam {
            device: "u1_gpio8".into(),
            param: param.into(),
            value,
        };
        analog.on_code(code, vec![set("v", v), set("r", r)]);
    }
    // Report the LED anode crossing 1 V (the LED starting or stopping to
    // conduct) to the host.
    analog.watch("v(led_a)", 1.0, Edge::Rising, HOST, LED_ON).unwrap();
    analog.watch("v(led_a)", 1.0, Edge::Falling, HOST, LED_OFF).unwrap();

    let mut soc = Esp32c3::new();
    soc.load_elf(&ElfImage::parse(BLINKY).unwrap()).unwrap();
    let mut cpu = Esp32c3Engine::new("esp32c3", soc);
    cpu.link_pin(PinLink { pin: 8, target: 1 });

    let mut k = Kernel::new(SimTime::from_us(50));
    let cpu = k.add_engine(Box::new(cpu));
    let an = k.add_engine(Box::new(analog));
    assert_eq!(an.0, 1);

    let mut crossings = Vec::new();
    let mut on_current = Vec::new();
    let end = SimTime::from_us(2_600);
    loop {
        match k.run_until(end).unwrap() {
            Stop::Host(ev) => {
                crossings.push((ev.at, ev.code));
                if ev.code == LED_ON {
                    // Sample the on-current 100 µs into each pulse.
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

    // GPIO edges as the firmware produced them.
    let soc = &k.engine_as::<Esp32c3Engine>(cpu).unwrap().soc;
    let gpio: Vec<_> = soc
        .pin_edges()
        .iter()
        .filter(|e| e.pin == 8)
        .skip(1)
        .map(|e| e.at)
        .collect();
    assert_eq!(soc.uart_text(), "ohmforge blinky\n");

    // Alternating on/off, one crossing per GPIO edge.
    assert_eq!(crossings.len(), gpio.len(), "crossings {crossings:?}");
    for (n, &(at, code)) in crossings.iter().enumerate() {
        assert_eq!(code, if n % 2 == 0 { LED_ON } else { LED_OFF });
        // The current follows the pin within a few nanoseconds (no
        // capacitance on the net beyond the LED's).
        let lag = at.0.checked_sub(gpio[n].0).expect("crossing after the edge");
        assert!(lag < 1_000_000, "crossing {n} lags the GPIO edge by {lag} ps");
    }
    // 500 µs period, 50 % duty.
    let on: Vec<_> = crossings.iter().filter(|c| c.1 == LED_ON).map(|c| c.0.0).collect();
    assert!(on.len() >= 5);
    for w in on.windows(2) {
        assert!(
            (w[1] - w[0]).abs_diff(500_000_000) < 1_000_000,
            "period {} ps",
            w[1] - w[0]
        );
    }
    // On-current: (3.3 V - V_F) / (330 Ω + 30 Ω pin), V_F about 1.8 to 2 V.
    assert!(!on_current.is_empty());
    for i in on_current {
        assert!((3.0e-3..4.5e-3).contains(&i), "LED on-current {i} A");
    }
}
