//! An unmodified esp-hal app through the whole ESP32-C3 boot path.
//!
//! `firmware/esp32c3-esp-hal-blinky` is an esp-hal 1.x blinky built and
//! merged by espflash. On the model the real mask ROM loads espflash's
//! ESP-IDF second-stage bootloader from flash offset 0; the bootloader
//! reads the partition table at `0x8000`, verifies the app at `0x10000`,
//! loads its RAM segments, maps its flash segments through the cache MMU
//! and jumps to it; the app raises the CPU clock to 160 MHz, prints a line
//! and toggles GPIO8 every 500 µs. The ROM and the bootloader are
//! Espressif's and fetched, not committed (`scripts/fetch-esp-rom-elfs.sh`;
//! see `common/mod.rs`): these tests skip without them unless
//! `OF_REQUIRE_ESP_ROM` is set.

mod common;

use of_analog::devices::PinDriver;
use of_analog::{AnalogEngine, Edge, EngineConfig, Options, SetParam};
use of_components::analog::{Board, Library, Placement, build};
use of_core::{HOST, Kernel, SimTime, Stop};
use of_soc_esp32c3::bootloader::espflash_flash_image;
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, PinLink, PinOut, all_pin_codes};

const APP: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-blinky/app.bin");
const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-blinky/partitions.bin");

/// The firmware's `HALF_PERIOD`, in µs.
const HALF_PERIOD_US: u64 = 500;
const HALF_PERIOD: SimTime = SimTime::from_us(HALF_PERIOD_US);
/// The app is running well before this (µs).
const APP_BY_US: u64 = 30_000;
const APP_BY: SimTime = SimTime::from_us(APP_BY_US);

/// What the ROM prints before it jumps to the bootloader.
const ROM_LOG: &str = "ESP-ROM:esp32c3-api1-20210207\r\n\
Build:Feb  7 2021\r\n\
rst:0x1 (POWERON),boot:0xc (SPI_FAST_FLASH_BOOT)\r\n\
SPIWP:0xee\r\n\
mode:DIO, clock div:2\r\n\
load:0x3fcd5830,len:0x1404\r\n\
load:0x403cbf10,len:0xc98\r\n\
load:0x403ce710,len:0x2f5c\r\n\
entry 0x403cbf1a\r\n";

/// The bootloader's log with its millisecond timestamps (`I (11) `)
/// dropped, then the app's line.
const BOOT_LOG: &[&str] = &[
    "boot: ESP-IDF v6.1-829-g9a97f6c54ec 2nd stage bootloader",
    "boot: chip revision: v0.3",
    "boot.esp32c3: SPI Speed      : 40MHz",
    "boot.esp32c3: SPI Mode       : DIO",
    "boot.esp32c3: SPI Flash Size : 4MB",
    "boot: Partition Table:",
    "boot: ## Label            Usage          Type ST Offset   Length",
    "boot:  0 nvs              WiFi data        01 02 00009000 00006000",
    "boot:  1 phy_init         RF data          01 01 0000f000 00001000",
    "boot:  2 factory          factory app      00 00 00010000 003f0000",
    "esp_image: segment 0: paddr=00010020 vaddr=3c000020 size=02278h (  8824) map",
    "esp_image: segment 1: paddr=000122a0 vaddr=3fc80bbc size=002c4h (   708) load",
    "esp_image: segment 2: paddr=0001256c vaddr=40380000 size=00bbch (  3004) load",
    "esp_image: segment 3: paddr=00013130 vaddr=00000000 size=0cee8h ( 52968) ",
    "esp_image: segment 4: paddr=00020020 vaddr=42010020 size=05e7ch ( 24188) map",
    "boot: Loaded app from partition at offset 0x10000",
];

/// Register blocks the bootloader and esp-hal touch that Ohmforge does not
/// model (analog, RF, RTC power and clock trimming, debug assist): reads
/// return 0 and writes are dropped. Anything outside these is a gap.
const UNMODELLED_OK: &[(u32, u32, &str)] = &[
    (0x6000_4f00, 0x100, "GPIO_SD"),
    (0x6000_5000, 0x1000, "FE2"),
    (0x6000_6000, 0x1000, "FE"),
    (0x6000_8000, 0x1000, "RTC_CNTL"),
    (0x6000_e000, 0x1000, "I2C_ANA_MST"),
    (0x6001_0000, 0x1000, "UART1"),
    (0x6001_c000, 0x1000, "NRX"),
    (0x6001_d000, 0x1000, "BB"),
    (0x6002_6000, 0x1000, "APB_CTRL"),
    (0x6004_0000, 0x1000, "APB_SARADC"), // beyond the one-shot registers
    (0x600c_0000, 0x1000, "SYSTEM"),
    (0x600c_1000, 0x1000, "SENSITIVE"),
    (0x600c_e000, 0x1000, "ASSIST_DEBUG"),
];

/// A C3 with the ROM, and the flash as `espflash flash` writes it, at
/// power-on; `None` if the Espressif files have not been fetched.
fn powered_on() -> Option<Esp32c3> {
    let (rom, bootloader) = (common::rom()?, common::bootloader()?);
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &espflash_flash_image(&bootloader).expect("bootloader image"));
    soc.write_flash(0x8000, PARTITIONS);
    soc.write_flash(0x1_0000, APP);
    soc.power_on_rom(&rom);
    Some(soc)
}

/// Drop the ESP-IDF log prefix `I (<ms>) ` from a line.
fn untimed(line: &str) -> &str {
    line.strip_prefix("I (")
        .and_then(|l| l.split_once(") "))
        .map_or(line, |(_, rest)| rest)
}

#[test]
fn rom_boots_the_idf_bootloader_which_boots_the_esp_hal_app() {
    let Some(mut soc) = powered_on() else { return };
    soc.run_until(APP_BY);
    let text = soc.uart_text();

    let rest = text.strip_prefix(ROM_LOG).unwrap_or_else(|| panic!("{text:?}"));
    let (boot, app) = rest.rsplit_once("\r\n").unwrap_or_else(|| panic!("{rest:?}"));
    let boot: Vec<_> = boot.split("\r\n").map(untimed).collect();
    assert_eq!(boot, BOOT_LOG);
    assert_eq!(app, "ohmforge esp-hal blinky\n");
    assert!(soc.resets().is_empty(), "{:?}", soc.resets());

    // esp-hal's CpuClock::max() is 160 MHz from the PLL; UART0 keeps
    // 115200 baud on the 80 MHz APB clock.
    assert_eq!(soc.cpu_hz(), 160_000_000);
    let baud = soc.bus.uart0.baud(80_000_000).expect("UART0 clock");
    assert!((baud - 115_200.0).abs() < 115_200.0 * 0.001, "{baud} baud");

    let gaps: Vec<_> = soc
        .bus
        .unmodelled()
        .keys()
        .filter(|&&a| {
            !UNMODELLED_OK
                .iter()
                .any(|&(base, len, _)| (base..base + len).contains(&a))
        })
        .collect();
    assert!(
        gaps.is_empty(),
        "unmodelled registers outside the known blocks: {gaps:#010x?}"
    );
}

#[test]
fn gpio8_toggles_at_the_firmware_period() {
    let Some(mut soc) = powered_on() else { return };
    soc.run_until(SimTime::from_us(APP_BY_US + 10 * HALF_PERIOD_US));
    let edges: Vec<_> = soc.pin_edges().iter().filter(|e| e.pin == 8).copied().collect();
    // Output enable drives the pin low first; the loop then toggles it,
    // starting high, every HALF_PERIOD.
    assert_eq!(edges[0].out, PinOut::driven(false, 2));
    assert!(
        edges[0].at > SimTime::from_ms(20) && edges[0].at < APP_BY,
        "{:?}",
        edges[0].at
    );
    assert!(edges.len() >= 14, "{} GPIO8 edges", edges.len());
    for (n, e) in edges[1..].iter().enumerate() {
        assert_eq!(e.out.level, Some(n % 2 == 0), "edge {n}");
    }
    for w in edges[1..].windows(2) {
        let dt = w[1].at.0 - w[0].at.0;
        assert!(
            dt.abs_diff(HALF_PERIOD.0) < SimTime::from_us(1).0,
            "half period {dt} ps"
        );
    }
    assert!(soc.resets().is_empty(), "{:?}", soc.resets());
}

#[test]
fn esp_hal_blinky_drives_the_led_circuit() {
    const VDD: f64 = 3.3;
    const R_SERIES: f64 = 330.0;
    const LED_ON: u32 = 1;
    const LED_OFF: u32 = 2;
    let Some(soc) = powered_on() else { return };

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
    analog.watch("v(led_a)", 1.0, Edge::Rising, HOST, LED_ON).unwrap();
    analog.watch("v(led_a)", 1.0, Edge::Falling, HOST, LED_OFF).unwrap();

    let mut cpu = Esp32c3Engine::new("esp32c3", soc);
    cpu.link_pin(PinLink { pin: 8, target: 1 });
    let mut k = Kernel::new(SimTime::from_us(50));
    let cpu = k.add_engine(Box::new(cpu));
    let an = k.add_engine(Box::new(analog));
    assert_eq!(an.0, 1);

    let mut crossings = Vec::new();
    let end = SimTime::from_us(APP_BY_US + 6 * HALF_PERIOD_US);
    loop {
        match k.run_until(end).unwrap() {
            Stop::Host(ev) => crossings.push((ev.at, ev.code)),
            Stop::Reached(_) => break,
            other => panic!("unexpected stop {other:?}"),
        }
    }

    let soc = &k.engine_as::<Esp32c3Engine>(cpu).unwrap().soc;
    assert!(soc.uart_text().ends_with("ohmforge esp-hal blinky\n"));
    let gpio: Vec<_> = soc
        .pin_edges()
        .iter()
        .filter(|e| e.pin == 8)
        .skip(1)
        .map(|e| e.at)
        .collect();
    // The LED stays dark through the ROM and the bootloader, then follows
    // every GPIO8 edge.
    assert_eq!(crossings.len(), gpio.len(), "crossings {crossings:?}");
    assert!(crossings.len() >= 6);
    for (n, &(at, code)) in crossings.iter().enumerate() {
        assert_eq!(code, if n % 2 == 0 { LED_ON } else { LED_OFF });
        let lag = at.0.checked_sub(gpio[n].0).expect("crossing after the edge");
        assert!(lag < 1_000_000, "crossing {n} lags the GPIO edge by {lag} ps");
    }
}
