//! GPIO inputs end to end: esp-hal firmware reads a push button that lives
//! in the analog circuit.
//!
//! `firmware/esp32c3-esp-hal-button` has two apps, booted the real way
//! (mask ROM, ESP-IDF bootloader, app; see `esp_hal_blinky.rs`): one polls
//! GPIO9 and copies it to the LED on GPIO8, the other toggles the LED from
//! a GPIO9 falling-edge interrupt and sleeps in `wfi`. GPIO9 has the
//! internal pull-up on; the parts-library push button `SW1` pulls it to
//! ground. The analog solver turns the pin voltage into input events
//! through V_IH/V_IL watches; the GPIO pull-up and the LED driver are pin
//! drivers that follow the firmware. Nothing debounces, so contact bounce
//! reaches the firmware. These tests skip without the fetched ROM and
//! bootloader unless `OF_REQUIRE_ESP_ROM` is set.

mod common;

use std::sync::OnceLock;

use of_analog::devices::{Capacitor, PinDriver, SourceValue, VoltageSource};
use of_analog::{AnalogEngine, Edge, EngineConfig, Options, SetParam};
use of_components::analog::{Board, Library, Placement, build};
use of_components::bounce::{PRESS_BOUNCE_US, RELEASE_BOUNCE_US};
use of_core::{Kernel, SimTime, Stop};
use of_soc_esp32c3::bootloader::espflash_flash_image;
use of_soc_esp32c3::{
    Esp32c3, Esp32c3Engine, PinLink, VIH_FRACTION, VIL_FRACTION, all_pin_codes, initial_input_level, input_code,
};

const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/partitions.bin");
const POLL: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/button_poll.bin");
const IRQ: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/button_irq.bin");

const VDD: f64 = 3.3;
const BUTTON: u8 = 9;
const LED: u8 = 8;
/// Event codes for the analog engine: close and open SW1.
const PRESS: u32 = 0x100;
const RELEASE: u32 = 0x101;

/// The app booted from power-on until it has printed its banner (shared by
/// the tests: booting through the ROM takes a few seconds of host time).
fn booted(app: &'static [u8], banner: &str) -> Option<Esp32c3> {
    let (rom, bootloader) = (common::rom()?, common::bootloader()?);
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &espflash_flash_image(&bootloader).expect("bootloader image"));
    soc.write_flash(0x8000, PARTITIONS);
    soc.write_flash(0x1_0000, app);
    soc.power_on_rom(&rom);
    let mut t = SimTime::ZERO;
    while !soc.uart_text().contains(banner) {
        assert!(t < SimTime::from_ms(100), "no banner: {:?}", soc.uart_text());
        t = t.saturating_add(SimTime::from_ms(1));
        soc.run_until(t);
    }
    // Let the app reach its loop.
    soc.run_until(t.saturating_add(SimTime::from_us(100)));
    assert!(soc.resets().is_empty(), "{:?}", soc.resets());
    Some(soc)
}

fn poll_app() -> Option<Esp32c3> {
    static SOC: OnceLock<Option<Esp32c3>> = OnceLock::new();
    SOC.get_or_init(|| booted(POLL, "ohmforge esp-hal button (polling)\n"))
        .clone()
}

fn irq_app() -> Option<Esp32c3> {
    static SOC: OnceLock<Option<Esp32c3>> = OnceLock::new();
    SOC.get_or_init(|| booted(IRQ, "ohmforge esp-hal button (interrupt)\n"))
        .clone()
}

/// The board around the chip.
#[derive(Clone, Copy)]
struct Setup {
    /// External pull-up on the button net (Ω), if any.
    pull_up: Option<f64>,
    /// Capacitor from the button net to ground (F), if any.
    cap: Option<f64>,
}

const BARE: Setup = Setup {
    pull_up: None,
    cap: None,
};

struct Run {
    /// LED pin levels and when they changed, from when the circuit was
    /// connected.
    led: Vec<(SimTime, bool)>,
    /// UART0 output since power-on.
    uart: String,
    kernel: Kernel,
}

/// Wire `soc` to the button board, press and release SW1 at `script`
/// times (relative to the start, `true` = closed), and run for `span`.
fn run(soc: Esp32c3, setup: Setup, script: &[(SimTime, bool)], span: SimTime) -> Run {
    let lib = Library::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")).unwrap();
    let mut placements = vec![
        Placement::new("SW1", "generic.pushbutton", &[("1", "gpio9"), ("2", "0")]),
        Placement::new("R2", "generic.resistor", &[("1", "gpio8"), ("2", "led_a")]).with("resistance", 330.0),
        Placement::new("D1", "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
    ];
    if let Some(r) = setup.pull_up {
        placements
            .push(Placement::new("R1", "generic.resistor", &[("1", "vdd"), ("2", "gpio9")]).with("resistance", r));
    }
    let mut b = build(
        &lib,
        &Board {
            placements,
            ..Board::default()
        },
    )
    .unwrap();
    let vdd = b.circuit.node("vdd");
    b.circuit
        .add(VoltageSource::new("vdd", vdd, 0, SourceValue::dc(VDD)))
        .unwrap();
    let btn = b.circuit.node("gpio9");
    if let Some(c) = setup.cap {
        b.circuit.add(Capacitor::new("c1", btn, 0, c, None)).unwrap();
    }
    // Each linked pin is a Thevenin driver that follows the chip.
    for pin in [BUTTON, LED] {
        let node = b.circuit.node(&format!("gpio{pin}"));
        let (v, r) = soc.pin(usize::from(pin)).thevenin(VDD);
        b.circuit
            .add(PinDriver::new(&format!("u1_gpio{pin}"), node, 0, v, r))
            .unwrap();
    }
    let press = b.switch_param("SW1", true).unwrap();
    let release = b.switch_param("SW1", false).unwrap();

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
    for pin in [BUTTON, LED] {
        for (code, out) in all_pin_codes(pin) {
            let (v, r) = out.thevenin(VDD);
            let set = |param: &str, value| SetParam {
                device: format!("u1_gpio{pin}"),
                param: param.into(),
                value,
            };
            analog.on_code(code, vec![set("v", v), set("r", r)]);
        }
    }
    analog.on_code(PRESS, vec![press]);
    analog.on_code(RELEASE, vec![release]);
    analog
        .watch_relative(
            "v(gpio9)",
            "v(vdd)",
            VIH_FRACTION,
            Edge::Rising,
            0,
            input_code(BUTTON, true),
        )
        .unwrap();
    analog
        .watch_relative(
            "v(gpio9)",
            "v(vdd)",
            VIL_FRACTION,
            Edge::Falling,
            0,
            input_code(BUTTON, false),
        )
        .unwrap();
    let v0 = analog.value("v(gpio9)").unwrap();

    let start = soc.hart.local_time();
    let mut cpu = Esp32c3Engine::new("esp32c3", soc);
    cpu.link_pin(PinLink { pin: BUTTON, target: 1 });
    cpu.link_pin(PinLink { pin: LED, target: 1 });
    cpu.link_input(BUTTON, initial_input_level(v0, VDD));
    let sent = cpu.soc.pin_edges().len();

    let mut k = Kernel::new(SimTime::from_us(50));
    assert_eq!(k.add_engine(Box::new(cpu)).0, 0);
    assert_eq!(k.add_engine(Box::new(analog)).0, 1);
    for &(at, closed) in script {
        k.schedule(start.saturating_add(at), 1, if closed { PRESS } else { RELEASE });
    }
    let end = start.saturating_add(span);
    assert_eq!(k.run_until(end).unwrap(), Stop::Reached(end));

    let soc = &k.engine_as::<Esp32c3Engine>(of_core::EngineId(0)).unwrap().soc;
    let led = soc.pin_edges()[sent..]
        .iter()
        .filter(|e| e.pin == LED)
        .map(|e| (SimTime(e.at.0 - start.0), e.out.level.expect("LED driven")))
        .collect();
    let uart = soc.uart_text();
    Run { led, uart, kernel: k }
}

fn us(t: f64) -> SimTime {
    SimTime::from_secs_f64(t * 1e-6)
}

fn analog(k: &mut Kernel) -> &mut AnalogEngine {
    k.engine_as::<AnalogEngine>(of_core::EngineId(1)).unwrap()
}

#[test]
fn polling_follows_the_button_within_a_loop_iteration() {
    let Some(soc) = poll_app() else { return };
    let script = [(us(200.0), true), (us(700.0), false)];
    let mut r = run(soc, BARE, &script, us(1_000.0));
    // LED on at the press, off at the release, nothing else.
    assert_eq!(
        r.led.iter().map(|e| e.1).collect::<Vec<_>>(),
        [true, false],
        "{:?}",
        r.led
    );
    for (&(at, _), &(t, _)) in script.iter().zip(&r.led) {
        // The input is read exactly (the read waits for the circuit), so
        // the LED follows within one pass of the loop, well under 2 µs.
        let lag = t.0.checked_sub(at.0).expect("LED after the button");
        assert!(lag < 2_000_000, "lag {lag} ps");
    }
    assert!(r.uart.ends_with("poll: 1 low\npoll: 2 high\n"), "{:?}", r.uart);
    // Released and pulled up by the internal pull-up only.
    let v = analog(&mut r.kernel).value("v(gpio9)").unwrap();
    assert!((v - VDD).abs() < 0.01, "{v} V");
}

#[test]
fn interrupt_wakes_the_sleeping_core_at_the_press() {
    let Some(soc) = irq_app() else { return };
    assert!(soc.is_sleeping(), "the app waits in wfi");
    let script = [(us(200.0), true), (us(500.0), false), (us(800.0), true)];
    let r = run(soc, BARE, &script, us(1_000.0));
    // Falling edges only: the LED toggles at each press, not the release.
    assert_eq!(
        r.led.iter().map(|e| e.1).collect::<Vec<_>>(),
        [true, false],
        "{:?}",
        r.led
    );
    for (&(at, _), &(t, _)) in [script[0], script[2]].iter().zip(&r.led) {
        // The sleeping core wakes at the crossing itself; what remains is
        // interrupt entry and esp-hal's handler dispatch.
        let lag = t.0.checked_sub(at.0).expect("LED after the button");
        assert!(lag < 5_000_000, "lag {lag} ps");
    }
    assert!(r.uart.ends_with("irq: 1\nirq: 2\n"), "{:?}", r.uart);
}

/// The firmware bounce tests use the canonical [`PRESS_BOUNCE_US`] /
/// [`RELEASE_BOUNCE_US`] trains, starting the press at 200 µs and the
/// release at 800 µs.
fn bouncy() -> Vec<(SimTime, bool)> {
    let mut script = Vec::new();
    for &(t, c) in PRESS_BOUNCE_US {
        script.push((us(200.0 + t), c));
    }
    for &(t, c) in RELEASE_BOUNCE_US {
        script.push((us(800.0 + t), c));
    }
    script
}

fn bouncy_levels() -> Vec<bool> {
    bouncy().into_iter().map(|(_, c)| c).collect()
}

#[test]
fn polling_sees_every_bounce() {
    let Some(soc) = poll_app() else { return };
    let r = run(soc, BARE, &bouncy(), us(1_000.0));
    // No debouncing: the LED repeats every contact change.
    let levels: Vec<_> = r.led.iter().map(|e| e.1).collect();
    assert_eq!(levels, bouncy_levels(), "{:?}", r.led);
    for (&(at, _), &(t, _)) in bouncy().iter().zip(&r.led) {
        assert!(t.0 - at.0 < 2_000_000, "{at:?} -> {t:?}");
    }
    assert!(r.uart.ends_with("poll: 8 high\n"), "{:?}", r.uart);
}

#[test]
fn edge_interrupts_count_every_bounce() {
    let Some(soc) = irq_app() else { return };
    let r = run(soc, BARE, &bouncy(), us(1_000.0));
    // Four closures (three on the press, one on the release): four toggles.
    let levels: Vec<_> = r.led.iter().map(|e| e.1).collect();
    assert_eq!(levels, [true, false, true, false], "{:?}", r.led);
    assert!(r.uart.ends_with("irq: 4\n"), "{:?}", r.uart);
}

#[test]
fn the_internal_pull_up_is_part_of_the_circuit() {
    let Some(soc) = poll_app() else { return };
    let pin = soc.pin(usize::from(BUTTON));
    assert!(pin.pull_up && !pin.pull_down && pin.level.is_none(), "{pin:?}");
    // Held down: about 45 kΩ from VDD into the 0.1 Ω contact.
    let mut r = run(soc, BARE, &[(us(100.0), true)], us(300.0));
    let a = analog(&mut r.kernel);
    let v = a.value("v(gpio9)").unwrap();
    let i = v / 0.1;
    assert!((i - VDD / 45e3).abs() < 1e-6, "{i} A through the button");
    assert_eq!(r.led.len(), 1);
}

#[test]
fn a_slow_edge_switches_at_v_ih() {
    let Some(soc) = poll_app() else { return };
    // 10 kΩ external pull-up with 100 nF: the release ramps up with
    // τ = (10 kΩ ∥ 45 kΩ) · 100 nF, and reads high only at V_IH = 0.75 VDD,
    // τ·ln 4 after the release (not at half VDD, τ·ln 2).
    let setup = Setup {
        pull_up: Some(10e3),
        cap: Some(100e-9),
    };
    let script = [(us(100.0), true), (us(300.0), false)];
    let r = run(soc, setup, &script, us(2_000.0));
    assert_eq!(
        r.led.iter().map(|e| e.1).collect::<Vec<_>>(),
        [true, false],
        "{:?}",
        r.led
    );
    let tau = 1.0 / (1.0 / 10e3 + 1.0 / 45e3) * 100e-9;
    let want = 300e-6 + tau * 4f64.ln();
    let got = r.led[1].0.as_secs_f64();
    assert!(
        (got - want).abs() < 0.01 * tau,
        "released high at {got} s, want {want} s"
    );
    // The press discharges the capacitor through the contact at once.
    assert!(r.led[0].0.0 - us(100.0).0 < 2_000_000, "{:?}", r.led[0]);
}
