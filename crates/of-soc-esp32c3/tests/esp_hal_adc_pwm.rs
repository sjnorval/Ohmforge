//! ADC and PWM end to end: esp-hal firmware reads a potentiometer and
//! dims an LED, both in the analog circuit.
//!
//! `firmware/esp32c3-esp-hal-adc-pwm` has two apps, booted the real way
//! (mask ROM, ESP-IDF bootloader, app; see `esp_hal_blinky.rs`). The pot
//! dimmer reads the wiper of the parts-library potentiometer `RV1` on GPIO2
//! (ADC1 channel 2, 11 dB, esp-hal curve calibration) and sets the LEDC
//! duty of the LED on GPIO8; the fade app ramps the LED up and down with
//! the LEDC's hardware fade. ADC conversions sample the circuit's node
//! voltage at the instant they start; every PWM edge is a pin event at the
//! exact counter tick. These tests skip without the fetched ROM and
//! bootloader unless `OF_REQUIRE_ESP_ROM` is set.

mod common;

use std::sync::OnceLock;

use of_analog::devices::{PinDriver, SourceValue, VoltageSource};
use of_analog::{AnalogEngine, EngineConfig, Options, SetParam};
use of_components::analog::{Board, Library, Placement, build};
use of_core::{Kernel, SimTime, Stop};
use of_soc_esp32c3::bootloader::espflash_flash_image;
use of_soc_esp32c3::saradc::{calibrated_mv, code_for};
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, PinLink, adc_request_code, adc_value_code, all_pin_codes};

const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/partitions.bin");
const DIMMER: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/pot_dimmer.bin");
const FADE: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/led_fade.bin");

const VDD: f64 = 3.3;
/// The pot wiper: GPIO2, ADC1 channel 2 (SAR ADC pad 2).
const POT_PAD: u8 = 2;
const LED: u8 = 8;
/// Event codes for the analog engine: turn RV1 to `POSITIONS[i]`.
const TURN: u32 = 0x200;
const POSITIONS: [f64; 4] = [0.1, 0.3, 0.6, 0.85];
/// 11 dB attenuation.
const ATTEN_11DB: usize = 3;

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
    assert!(soc.resets().is_empty(), "{:?}", soc.resets());
    Some(soc)
}

fn dimmer_app() -> Option<Esp32c3> {
    static SOC: OnceLock<Option<Esp32c3>> = OnceLock::new();
    SOC.get_or_init(|| booted(DIMMER, "ohmforge esp-hal pot dimmer\n"))
        .clone()
}

fn fade_app() -> Option<Esp32c3> {
    static SOC: OnceLock<Option<Esp32c3>> = OnceLock::new();
    SOC.get_or_init(|| booted(FADE, "ohmforge esp-hal led fade\n")).clone()
}

struct Run {
    /// LED pin levels and when they changed, from when the circuit was
    /// connected.
    led: Vec<(SimTime, bool)>,
    /// UART0 output since power-on.
    uart: String,
    /// UART0 output while the circuit was connected.
    new_uart: String,
    kernel: Kernel,
}

/// Wire `soc` to the board (pot RV1 from ground to VDD with its wiper on
/// GPIO2 at `POSITIONS[0]`, LED with 330 Ω on GPIO8), turn RV1 to
/// `POSITIONS[i]` at each `turns[i - 1]` (relative to the start) and run
/// for `span`.
fn run(soc: Esp32c3, turns: &[SimTime], span: SimTime) -> Run {
    let lib = Library::load_dir(&std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")).unwrap();
    let placements = vec![
        Placement::new("RV1", "bourns.ptv09a-b103", &[("1", "0"), ("W", "gpio2"), ("3", "vdd")])
            .with("position", POSITIONS[0]),
        Placement::new("R2", "generic.resistor", &[("1", "gpio8"), ("2", "led_a")]).with("resistance", 330.0),
        Placement::new("D1", "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
    ];
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
    let node = b.circuit.node("gpio8");
    let (v, r) = soc.pin(usize::from(LED)).thevenin(VDD);
    b.circuit.add(PinDriver::new("u1_gpio8", node, 0, v, r)).unwrap();
    let turn_params: Vec<Vec<SetParam>> = POSITIONS.iter().map(|&x| b.pot_params("RV1", x).unwrap()).collect();

    let mut analog = AnalogEngine::new(
        "analog",
        b.circuit,
        Options::default(),
        EngineConfig {
            max_step: 20e-6,
            ..EngineConfig::default()
        },
    )
    .unwrap();
    for (code, out) in all_pin_codes(LED) {
        let (v, r) = out.thevenin(VDD);
        let set = |param: &str, value| SetParam {
            device: "u1_gpio8".into(),
            param: param.into(),
            value,
        };
        analog.on_code(code, vec![set("v", v), set("r", r)]);
    }
    for (i, p) in turn_params.into_iter().enumerate() {
        analog.on_code(TURN + i as u32, p);
    }
    analog
        .sample_on(adc_request_code(POT_PAD), "v(gpio2)", 0, adc_value_code(POT_PAD))
        .unwrap();

    let start = soc.hart.local_time();
    let uart_before = soc.uart_text().len();
    let mut cpu = Esp32c3Engine::new("esp32c3", soc);
    cpu.link_pin(PinLink { pin: LED, target: 1 });
    cpu.link_adc(POT_PAD, 1);
    let sent = cpu.soc.pin_edges().len();

    let mut k = Kernel::new(SimTime::from_us(50));
    assert_eq!(k.add_engine(Box::new(cpu)).0, 0);
    assert_eq!(k.add_engine(Box::new(analog)).0, 1);
    for (i, &at) in turns.iter().enumerate() {
        k.schedule(start.saturating_add(at), 1, TURN + i as u32 + 1);
    }
    let end = start.saturating_add(span);
    assert_eq!(k.run_until(end).unwrap(), Stop::Reached(end));

    let soc = &k.engine_as::<Esp32c3Engine>(of_core::EngineId(0)).unwrap().soc;
    assert!(soc.resets().is_empty(), "{:?}", soc.resets());
    let led = soc.pin_edges()[sent..]
        .iter()
        .filter(|e| e.pin == LED)
        .map(|e| (SimTime(e.at.0 - start.0), e.out.level.expect("LED driven")))
        .collect();
    let uart = soc.uart_text();
    let new_uart = uart[uart_before..].to_owned();
    Run {
        led,
        uart,
        new_uart,
        kernel: k,
    }
}

fn ms(t: f64) -> SimTime {
    SimTime::from_secs_f64(t * 1e-3)
}

/// High time over period of the last whole PWM period (rising edge,
/// falling edge, rising edge) before `before`; with no edges in the
/// millisecond before, 0 % or 100 % from the level.
fn duty_before(led: &[(SimTime, bool)], before: SimTime) -> f64 {
    let seen: Vec<&(SimTime, bool)> = led.iter().filter(|e| e.0 < before).collect();
    let n = seen.len();
    if n == 0 || seen[n - 1].0.0 + ms(1.0).0 < before.0 {
        return if seen.last().is_some_and(|e| e.1) { 1.0 } else { 0.0 };
    }
    // The last rising edge, and the period ending there.
    let end = (0..n).rev().find(|&i| seen[i].1).expect("a rising edge");
    assert!(end >= 2 && seen[end - 2].1 && !seen[end - 1].1, "{seen:?}");
    let (rise, fall) = (seen[end - 2].0.0, seen[end - 1].0.0);
    (fall - rise) as f64 / (seen[end].0.0 - rise) as f64
}

#[test]
fn the_dimmer_reads_the_pot_and_sets_the_led_duty() {
    let Some(soc) = dimmer_app() else { return };
    let cal = soc.bus.saradc.cal_code(ATTEN_11DB);
    let turns = [ms(30.0), ms(60.0), ms(90.0)];
    let mut r = run(soc, &turns, ms(120.0));
    assert!(!r.uart.contains("panic"), "{:?}", r.uart);

    // Readings: `adc: <mV> duty: <duty>` every 5 ms.
    let readings: Vec<(u32, u32)> = r
        .new_uart
        .lines()
        .filter_map(|l| {
            let rest = l.strip_prefix("adc: ")?;
            let (mv, duty) = rest.split_once(" duty: ")?;
            Some((mv.parse().ok()?, duty.parse().ok()?))
        })
        .collect();
    assert!(readings.len() >= 20, "{:?}", r.new_uart);
    for (i, &x) in POSITIONS.iter().enumerate() {
        // The unloaded wiper: x · VDD. The model's code for it, through
        // esp-hal's calibration, recovers it within a few mV; the datasheet
        // allows ±35 mV at 11 dB over 0 to 2500 mV.
        let volts = x * VDD;
        let want = u32::from(calibrated_mv(code_for(volts, ATTEN_11DB, cal), ATTEN_11DB, cal));
        assert!(
            (f64::from(want) - volts * 1e3).abs() < 5.0,
            "model: {want} mV for {volts} V"
        );
        let seen: Vec<u32> = readings.iter().map(|r| r.0).collect();
        assert!(seen.contains(&want), "position {x}: want {want} mV in {seen:?}");
        let duty = want.min(2500) * 1024 / 2500;
        assert!(readings.contains(&(want, duty)), "{readings:?}");
        // The PWM a few ms after the turn (the next reading, plus one PWM
        // period to latch the duty) shows that duty on the LED.
        let at = turns.get(i).copied().unwrap_or(ms(120.0));
        // Within one 12.5 ns APB clock: the 78.125 divider makes ticks of
        // 78 and 79 clocks.
        let got = duty_before(&r.led, SimTime(at.0 - ms(1.0).0));
        assert!(
            (got - f64::from(duty) / 1024.0).abs() < 1.25e-5,
            "position {x}: duty {got}, want {duty}/1024"
        );
    }
    // 1 kHz exactly: 80 MHz / 78.125 / 1024 (a fractional divider).
    let rises: Vec<SimTime> = r.led.iter().filter(|e| e.1).map(|e| e.0).collect();
    for w in rises.windows(2).filter(|w| w[0] > ms(5.0) && w[1] < ms(85.0)) {
        assert_eq!(w[1].0 - w[0].0, ms(1.0).0, "period at {:?}", w[0]);
    }
    // Every LEDC and one-shot ADC register the app touches is modelled.
    let soc = &r.kernel.engine_as::<Esp32c3Engine>(of_core::EngineId(0)).unwrap().soc;
    let gaps: Vec<_> = soc
        .bus
        .unmodelled()
        .keys()
        .filter(|&&a| (0x6001_9000..0x6001_a000).contains(&a) || (0x6004_0000..0x6004_0064).contains(&a))
        .collect();
    assert!(gaps.is_empty(), "unmodelled LEDC/ADC registers: {gaps:#010x?}");
    // Above 2500 mV the duty is 1024/1024: always high, no edges, and the
    // LED conducts.
    let a = r.kernel.engine_as::<AnalogEngine>(of_core::EngineId(1)).unwrap();
    let led_v = a.value("v(led_a)").unwrap();
    assert!(led_v > 1.5, "v(led_a) = {led_v} V");
    let last_turn = turns[2];
    assert!(
        r.led.iter().filter(|e| e.0 > SimTime(last_turn.0 + ms(8.0).0)).count() == 0,
        "{:?}",
        &r.led[r.led.len().saturating_sub(4)..]
    );
}

/// LEDC timer ticks (of 256 per period) the LED is high in each PWM
/// period from the first rising edge to the last, with each period's
/// start; periods with no rising edge (0 %) count 0.
fn high_ticks(led: &[(SimTime, bool)], period: SimTime) -> Vec<(SimTime, u32)> {
    let rises: Vec<usize> = (0..led.len()).filter(|&i| led[i].1).collect();
    let mut out = Vec::new();
    for w in rises.windows(2) {
        let (rise, fall) = (led[w[0]].0, led[w[0] + 1].0);
        let gap = led[w[1]].0.0 - rise.0;
        assert_eq!(gap % period.0, 0, "period at {rise:?}");
        let ticks = (fall.0 - rise.0) as f64 * 256.0 / period.0 as f64;
        out.push((rise, ticks.round() as u32));
        for k in 1..gap / period.0 {
            out.push((SimTime(rise.0 + k * period.0), 0));
        }
    }
    out
}

#[test]
fn the_ledc_fades_the_led_up_and_down_in_hardware() {
    let Some(soc) = fade_app() else { return };
    let mut r = run(soc, &[], ms(110.0));
    assert!(!r.uart.contains("panic"), "{:?}", r.uart);
    assert!(r.new_uart.contains("fade: 1 up\nfade: 2 down\n"), "{:?}", r.new_uart);

    // 5 kHz exactly: 80 MHz / 62.5 / 256.
    let period = SimTime::from_us(200);
    let highs = high_ticks(&r.led, period);
    // esp-hal asks for 255 steps of +1 (of 256), one per PWM period, from
    // 0: the duty rises by one tick each period to 255, then falls back the
    // same way. (The first few steps may have passed before the circuit
    // was connected, up to 1 ms after the banner.)
    let ticks: Vec<u32> = highs.iter().map(|h| h.1).collect();
    let peak = ticks.iter().position(|&t| t == 255).expect("reaches 255/256");
    assert!(ticks[0] <= 6, "{:?}", &ticks[..4]);
    let up: Vec<u32> = (ticks[0]..=255).collect();
    assert_eq!(ticks[..=peak], up[..], "fade up");
    // Down: the CPU sees the fade end during the 255 period and starts the
    // next fade from 255 (255 · 100 / 100). Fade settings take effect at
    // once (TRM 32.3.4), so the first step already lands at the next
    // overflow: one tick less per period down to 0.
    let bottom = peak + ticks[peak..].iter().position(|&t| t == 0).expect("back to 0");
    let down: Vec<u32> = (0..=255).rev().collect();
    assert_eq!(ticks[peak..=bottom], down[..], "fade down");
    // Each fade takes 255 periods (51 ms), not the 100 ms asked for:
    // esp-hal rounds 500 cycles / 255 steps down to one cycle per step.
    assert_eq!(highs[bottom].0.0 - highs[peak].0.0, 255 * period.0);

    // The LED in the circuit follows: at the last edge it conducts while
    // the pin is high.
    let a = r.kernel.engine_as::<AnalogEngine>(of_core::EngineId(1)).unwrap();
    let led_v = a.value("v(led_a)").unwrap();
    let pin_high = r.led.last().is_some_and(|e| e.1);
    assert_eq!(led_v > 1.5, pin_high, "v(led_a) = {led_v} V");
}
