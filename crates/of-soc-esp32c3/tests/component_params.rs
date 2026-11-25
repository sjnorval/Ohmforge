//! E4b: `Esp32c3Component::from_spec` params and boot paths (engine plan
//! Task 11). The ROM case skips without the fetched ROM unless
//! `OF_REQUIRE_ESP_ROM` is set.

mod common;

use std::collections::BTreeMap;

use of_component_api::ParamValue;
use of_core::SimTime;
use of_engine::{ComponentSpec, EngineBuilder, Firmware, Registry};
use of_soc_esp32c3::bootloader::{C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256};
use of_soc_esp32c3::rom::{C3_ROM_FILE, C3_ROM_SHA256};
use of_soc_esp32c3::{Esp32c3, Esp32c3Component, QUIESCENT_A, register};

const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");
const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/partitions.bin");
const DIMMER: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/pot_dimmer.bin");

fn pins(names: &[&str]) -> Vec<String> {
    names.iter().map(|s| (*s).to_owned()).collect()
}

fn spec<'a>(params: &'a BTreeMap<String, ParamValue>, pins: &'a [String], fw: Option<Firmware>) -> ComponentSpec<'a> {
    ComponentSpec {
        refdes: "U1",
        part_id: "espressif.esp32-c3",
        params,
        firmware: fw,
        pins,
        seed: 0,
    }
}

fn device_names(chip: Esp32c3Component) -> Vec<String> {
    let mut eng = EngineBuilder::new("gnd");
    eng.net("vdd").net("gpio8");
    // A resistor so no net is left with a single pin.
    let r = of_components::Resistor::new("R1", 1e3);
    eng.add(Box::new(r), &[("1", "gpio8"), ("2", "vdd")], &[], 0).unwrap();
    eng.add(
        Box::new(chip),
        &[("3V3", "vdd"), ("GND", "gnd"), ("GPIO8", "gpio8")],
        &[],
        0,
    )
    .unwrap();
    eng.elaborate().unwrap().device_names
}

#[test]
fn quiescent_load_is_a_param_and_zero_adds_no_device() {
    let p = pins(&["3V3", "GND", "GPIO8"]);
    let mut params = BTreeMap::new();
    let chip = Esp32c3Component::from_spec(spec(&params, &p, Some(Firmware::Elf(BLINKY.to_vec())))).unwrap();
    assert_eq!(chip.quiescent_a(), QUIESCENT_A);
    assert!(device_names(chip).iter().any(|d| d == "u1_iq"));

    params.insert("quiescent_a".into(), ParamValue::Number(0.0));
    let chip = Esp32c3Component::from_spec(spec(&params, &p, Some(Firmware::Elf(BLINKY.to_vec())))).unwrap();
    assert_eq!(chip.quiescent_a(), 0.0);
    assert!(!device_names(chip).iter().any(|d| d == "u1_iq"));
}

#[test]
fn bad_params_and_missing_firmware_are_errors() {
    let p = pins(&["3V3", "GND", "GPIO8"]);
    let none = BTreeMap::new();
    let err = Esp32c3Component::from_spec(spec(&none, &p, None)).err().unwrap();
    assert!(err.to_string().contains("no firmware"), "{err}");

    let cases = [
        ("inputs", ParamValue::List(vec!["GPIO9".into()]), "not a connected GPIO"),
        ("cpu_mhz", ParamValue::Number(80.0), "only 160"),
        ("quiescent_a", ParamValue::Number(-1.0), ">= 0"),
    ];
    for (k, v, want) in cases {
        let params = BTreeMap::from([(k.to_owned(), v)]);
        let err = Esp32c3Component::from_spec(spec(&params, &p, Some(Firmware::Elf(BLINKY.to_vec()))))
            .err()
            .unwrap_or_else(|| panic!("{k} should fail"));
        assert!(err.to_string().contains(want), "{k}: {err}");
    }
    let wrong = Firmware::Booted(Box::new(42u32));
    let err = Esp32c3Component::from_spec(spec(&none, &p, Some(wrong))).err().unwrap();
    assert!(err.to_string().contains("not an ESP32-C3"), "{err}");
}

#[test]
fn registry_builds_the_chip_with_connected_pins_only() {
    let mut reg = Registry::new();
    register(&mut reg);
    let p = pins(&["GPIO9", "3V3", "GPIO8", "GND"]);
    let params = BTreeMap::from([("inputs".to_owned(), ParamValue::List(vec!["GPIO9".into()]))]);
    let c = reg
        .build("rust:esp32c3", spec(&params, &p, Some(Firmware::Elf(BLINKY.to_vec()))))
        .unwrap();
    let names: Vec<&str> = c.info().pins.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["3V3", "GND", "GPIO9", "GPIO8"]);
    assert!(c.has_engine());
}

#[test]
fn rom_boot_reaches_the_dimmer_banner() {
    let (Some(rom), Some(bootloader)) = (
        common::fetched(C3_ROM_FILE, C3_ROM_SHA256),
        common::fetched(C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256),
    ) else {
        return;
    };
    let p = pins(&["3V3", "GND", "GPIO2", "GPIO8"]);
    let params = BTreeMap::new();
    let fw = Firmware::Rom {
        app: DIMMER.to_vec(),
        partitions: PARTITIONS.to_vec(),
        rom,
        bootloader,
        ready_text: Some("ohmforge esp-hal pot dimmer\n".into()),
    };
    let chip = Esp32c3Component::from_spec(spec(&params, &p, Some(fw))).unwrap();
    let soc: &Esp32c3 = chip.soc().expect("booted");
    assert!(soc.uart_text().contains("ohmforge esp-hal pot dimmer"));
    // Booted chips start the session at their own clock.
    assert!(soc.hart.local_time() > SimTime::ZERO);
}
