//! Circuit file -> registry -> engine (feature `engine`).
#![cfg(feature = "engine")]

use std::sync::Arc;

use of_circuit::build::{build, build_engine};
use of_circuit::embedded::EmbeddedResolver;
use of_circuit::library::{PartsIndex, sha256_hex};
use of_circuit::parse;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::SimTime;
use of_engine::Registry;

fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

fn parts() -> PartsIndex {
    PartsIndex::new(of_parts::load_dir(&parts_dir()).unwrap().into_iter().map(|(_, p)| p))
}

fn registry() -> Registry {
    let lib = Arc::new(Library::load_dir(&parts_dir()).unwrap());
    let mut reg = Registry::new();
    register_builtin(&mut reg, lib, Corner::Typical, Env::default());
    of_soc_esp32c3::register(&mut reg);
    reg
}

const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");

/// LED + 330 ohm + bench PSU, no MCU.
const LED: &str = r#"format = 1

[circuit]
name = "LED"

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 330 }

[[component]]
ref = "D1"
part = "generic.led-red-0603"

[[component]]
ref = "PSU"
part = "bench.psu"
params = { volts = 3.3 }

[[net]]
name = "vdd"
kind = "power"
pins = ["PSU.+", "R1.1"]

[[net]]
name = "anode"
pins = ["R1.2", "D1.A"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["D1.K", "PSU.-"]
"#;

fn blink(sha: &str, gpio: &str) -> String {
    format!(
        r#"format = 1

[circuit]
name = "Blink"

[[component]]
ref = "U1"
part = "espressif.esp32-c3"
params = {{ quiescent_a = 0 }}

[component.firmware]
app = "fw/blinky.elf"
app_sha256 = "{sha}"
boot = "direct"

[[component]]
ref = "R1"
part = "generic.resistor"
params = {{ resistance = 330 }}

[[component]]
ref = "D1"
part = "generic.led-red-0603"

[[component]]
ref = "PSU"
part = "bench.psu"

[[net]]
name = "gpio8"
pins = ["U1.{gpio}", "R1.1"]

[[net]]
name = "led_a"
pins = ["R1.2", "D1.A"]

[[net]]
name = "vdd"
kind = "power"
pins = ["U1.3V3", "PSU.+"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["U1.GND", "D1.K", "PSU.-"]
"#
    )
}

fn fw() -> EmbeddedResolver {
    EmbeddedResolver::new().with_file("fw/blinky.elf", BLINKY)
}

#[test]
fn led_circuit_builds_and_holds_its_operating_point() {
    let f = parse(LED).unwrap();
    let mut b = build(LED, &f, &parts(), &registry(), &EmbeddedResolver::new()).unwrap();
    let v0 = b.session.value("v(anode)").unwrap();
    assert!((1.6..2.2).contains(&v0), "LED forward voltage {v0}");
    b.session.run_until(SimTime::from_ms(1)).unwrap();
    let v1 = b.session.value("v(anode)").unwrap();
    assert!((v1 - v0).abs() < 1e-9, "{v0} -> {v1}");
    assert_eq!(b.file.components.len(), 3);
}

#[test]
fn ground_defaults_to_gnd() {
    let f = parse(LED).unwrap();
    assert_eq!(f.circuit.ground.get_ref(), "gnd");
    assert!(build_engine(LED, &f, &parts(), &registry(), &EmbeddedResolver::new()).is_ok());
}

#[test]
fn blinky_runs_from_the_file_and_prints_its_banner() {
    let src = blink(&sha256_hex(BLINKY), "GPIO8");
    let f = parse(&src).unwrap();
    let mut b = build(&src, &f, &parts(), &registry(), &fw()).unwrap();
    b.session.run_until(SimTime::from_ms(3)).unwrap();
    let cpu = b
        .session
        .engine_as::<of_soc_esp32c3::Esp32c3Engine>("U1")
        .expect("U1 engine");
    assert!(
        cpu.soc.uart_text().contains("ohmforge blinky"),
        "{:?}",
        cpu.soc.uart_text()
    );
}

#[test]
fn unknown_part_reports_line_and_column() {
    let src = LED.replace("generic.led-red-0603", "acme.flux");
    let f = parse(&src).unwrap();
    let e = build_engine(&src, &f, &parts(), &registry(), &EmbeddedResolver::new())
        .err()
        .unwrap();
    let line = src.lines().position(|l| l.contains("acme.flux")).unwrap() + 1;
    assert_eq!((e.line, e.column), (line, 8), "{e}");
    assert!(e.message.contains("acme.flux"), "{e}");
}

#[test]
fn unknown_pin_reports_line_and_column() {
    let src = blink(&sha256_hex(BLINKY), "GPIO99");
    let f = parse(&src).unwrap();
    let e = build_engine(&src, &f, &parts(), &registry(), &fw()).err().unwrap();
    let line = src.lines().position(|l| l.contains("U1.GPIO99")).unwrap() + 1;
    assert_eq!(e.line, line, "{e}");
    assert!(e.message.contains("GPIO99"), "{e}");
}

#[test]
fn bad_firmware_hash_reports_the_actual_hash() {
    let src = blink(&"0".repeat(64), "GPIO8");
    let f = parse(&src).unwrap();
    let e = build_engine(&src, &f, &parts(), &registry(), &fw()).err().unwrap();
    let line = src.lines().position(|l| l.starts_with("app_sha256")).unwrap() + 1;
    assert_eq!((e.line, e.column), (line, 14), "{e}");
    assert!(e.message.contains(&sha256_hex(BLINKY)), "{e}");
}

#[test]
fn unregistered_model_names_the_ref_and_the_model() {
    let src = LED.to_owned();
    let f = parse(&src).unwrap();
    let e = build_engine(&src, &f, &parts(), &Registry::new(), &EmbeddedResolver::new())
        .err()
        .unwrap();
    assert!(e.message.contains("R1") && e.message.contains("rust:resistor"), "{e}");
    assert!(e.line > 0, "{e}");
}

#[test]
fn rom_boot_without_a_rom_says_why() {
    let sha = sha256_hex(BLINKY);
    let src = blink(&sha, "GPIO8").replace(
        "boot = \"direct\"",
        &format!("boot = \"rom\"\npartitions = \"fw/blinky.elf\"\npartitions_sha256 = \"{sha}\"\nready_text = \"x\""),
    );
    let f = parse(&src).unwrap();
    let e = build_engine(&src, &f, &parts(), &registry(), &fw()).err().unwrap();
    assert!(e.message.contains("U1") && e.message.contains("mask ROM"), "{e}");
}

#[test]
fn a_non_gnd_ground_is_reported_at_the_ground_name() {
    let src = LED
        .replace("name = \"LED\"", "name = \"LED\"\nground = \"vss\"")
        .replace("name = \"gnd\"", "name = \"vss\"");
    let f = parse(&src).unwrap();
    let e = build_engine(&src, &f, &parts(), &registry(), &EmbeddedResolver::new())
        .err()
        .unwrap();
    assert_eq!(e.line, 5, "{e}");
    assert!(e.message.contains("vss"), "{e}");
}
