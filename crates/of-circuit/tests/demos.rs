//! The shipped demo circuits (`circuits/demos`, features `demos` and `fs`).
#![cfg(all(feature = "demos", feature = "fs"))]

use std::path::Path;

use of_circuit::demos::{DEMOS, FIRMWARE, MODELS, PARTS, demo, part_files, parts, resolver, spice_model};
use of_circuit::fs::{FsResolver, load_parts};
use of_circuit::library::{check_firmware, check_parts, check_signals, normalize};
use of_circuit::{BootKind, parse, save};

/// The 555 demos (no microcontroller).
fn is_555(name: &str) -> bool {
    name.starts_with("ne555-")
}

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn the_demos_are_listed_and_the_embedded_text_is_the_file() {
    let names: Vec<_> = DEMOS.iter().map(|d| d.name).collect();
    assert_eq!(
        names,
        [
            "blink",
            "dimmer",
            "fade",
            "button",
            "button-irq",
            "ne555-astable",
            "ne555-monostable",
            "ne555-pwm-dimmer",
            "esp32c3-555-clock"
        ]
    );
    for d in DEMOS {
        let on_disk = std::fs::read_to_string(repo().join(d.path)).unwrap();
        assert_eq!(
            on_disk.replace("\r\n", "\n"),
            d.text.replace("\r\n", "\n"),
            "{}",
            d.path
        );
    }
    assert!(demo("fade").is_some() && demo("nope").is_none());
}

#[test]
fn demos_are_canonical_and_match_the_parts_library() {
    let lib = load_parts(&repo().join("parts")).unwrap();
    for d in DEMOS {
        let mut f = parse(d.text).unwrap_or_else(|e| panic!("{}: {e}", d.path));
        let problems = check_parts(d.text, &f, &lib);
        assert!(problems.is_empty(), "{}: {problems:?}", d.path);
        let problems = check_signals(d.text, &f, &lib);
        assert!(problems.is_empty(), "{}: {problems:?}", d.path);
        normalize(&mut f, Some(&lib));
        assert_eq!(save(&f), d.text.replace("\r\n", "\n"), "{} is not canonical", d.path);
        assert_eq!(f.circuit.ground.get_ref(), "gnd", "{}", d.path);
    }
}

#[test]
fn every_demo_has_an_ideal_ammeter_and_scope_on_the_led_current() {
    for d in DEMOS {
        let f = parse(d.text).unwrap();
        if is_555(d.name) {
            // The 555 demos meter the LED current and scope their timing
            // capacitor (and trigger) with the output.
            let meter = f.instruments.iter().find(|i| i.reference() == "XMM1").unwrap();
            assert_eq!((meter.kind(), meter.mode()), ("multimeter", Some("dc_a")), "{}", d.path);
            let probes: Vec<&str> = meter.probes.iter().map(|p| p.get_ref().as_str()).collect();
            assert_eq!(probes, ["LED current"], "{}", d.path);
            let scope = f.instruments.iter().find(|i| i.reference() == "XSC1").unwrap();
            assert_eq!(scope.kind(), "scope", "{}", d.path);
            assert!(scope.probes.iter().any(|p| p.get_ref() == "Output"), "{}", d.path);
            continue;
        }
        if d.name == "esp32c3-555-clock" {
            // The clock counter scopes the clock and the timing capacitor,
            // and reads the clock and TX on a logic analyser and the console.
            let kinds: Vec<(&str, &str)> = f.instruments.iter().map(|i| (i.reference(), i.kind())).collect();
            assert_eq!(kinds, [("XSC1", "scope"), ("XLA1", "logic"), ("XU1", "uart")]);
            let probes = |r: &str| -> Vec<String> {
                let i = f.instruments.iter().find(|i| i.reference() == r).unwrap();
                i.probes.iter().map(|p| p.get_ref().clone()).collect()
            };
            assert_eq!(probes("XSC1"), ["Clock at GPIO4", "Timing capacitor"]);
            assert_eq!(probes("XLA1"), ["d(U1.GPIO4)", "d(U1.GPIO21)"]);
            assert_eq!(probes("XU1"), ["d(U1.GPIO21)"]);
            continue;
        }
        let kinds: Vec<(&str, &str, Option<&str>)> = f
            .instruments
            .iter()
            .map(|i| (i.reference(), i.kind(), i.mode()))
            .collect();
        assert_eq!(
            kinds,
            [("XMM1", "multimeter", Some("dc_a")), ("XSC1", "scope", None)],
            "{}",
            d.path
        );
        for i in &f.instruments {
            assert_eq!(i.profile(), "ideal", "{}", d.path);
            let probes: Vec<&str> = i.probes.iter().map(|p| p.get_ref().as_str()).collect();
            assert_eq!(probes, ["LED current"], "{}", d.path);
        }
    }
}

#[test]
fn demo_firmware_hashes_match_on_disk_and_embedded() {
    let disk = FsResolver::for_file(&repo().join("circuits/demos/blink.circuit.toml"));
    let mem = resolver();
    for d in DEMOS {
        let f = parse(d.text).unwrap();
        assert!(check_firmware(d.text, &f, &disk).is_empty(), "{}", d.path);
        assert!(check_firmware(d.text, &f, &mem).is_empty(), "{}", d.path);
    }
    assert_eq!(mem.paths().count(), FIRMWARE.len());
}

#[test]
fn shipped_demos_keep_the_idle_draw_and_name_their_boot() {
    for d in DEMOS {
        let f = parse(d.text).unwrap();
        if is_555(d.name) {
            // No microcontroller, so no firmware.
            assert!(f.components.iter().all(|c| c.firmware.is_none()), "{}", d.path);
            continue;
        }
        let u1 = f.components.iter().find(|c| c.reference() == "U1").unwrap();
        assert_eq!(
            u1.params.get("quiescent_a"),
            // ESP32-C3 datasheet v2.5, Table 5-8, 160 MHz: 23 mA with the CPU
            // running, 16 mA for button-irq and the clock counter, which
            // sleep in `wfi`.
            Some(&of_circuit::ParamValue::Number(
                if matches!(d.name, "button-irq" | "esp32c3-555-clock") {
                    0.016
                } else {
                    0.023
                }
            )),
            "{}",
            d.path
        );
        let fw = u1.firmware.as_ref().unwrap();
        // Our own assembly firmware boots directly, with no ROM.
        let want = if matches!(d.name, "blink" | "esp32c3-555-clock") {
            BootKind::Direct
        } else {
            BootKind::Rom
        };
        assert_eq!(fw.boot, want, "{}", d.path);
        if want == BootKind::Rom {
            assert!(
                fw.ready_text.as_deref().is_some_and(|t| t.ends_with('\n')),
                "{}",
                d.path
            );
            assert!(fw.partitions_sha256.is_some(), "{}", d.path);
        }
    }
}

#[test]
fn embedded_parts_are_the_library_files_and_cover_every_demo() {
    for (dir, text) in PARTS {
        let on_disk = std::fs::read_to_string(repo().join("parts").join(dir).join("part.toml")).unwrap();
        assert_eq!(on_disk.replace("\r\n", "\n"), text.replace("\r\n", "\n"), "{dir}");
    }
    let lib = parts().unwrap();
    assert_eq!(lib.len(), PARTS.len());
    for d in DEMOS {
        let f = parse(d.text).unwrap();
        let problems = check_parts(d.text, &f, &lib);
        assert!(problems.is_empty(), "{}: {problems:?}", d.path);
    }
}

#[test]
fn embedded_spice_models_are_the_library_files_and_cover_every_part() {
    for (dir, file, text) in MODELS {
        let on_disk = std::fs::read_to_string(repo().join("parts").join(dir).join(file)).unwrap();
        assert_eq!(
            on_disk.replace("\r\n", "\n"),
            text.replace("\r\n", "\n"),
            "{dir}/{file}"
        );
    }
    for p in part_files().unwrap() {
        let model = &p.part.model;
        assert_eq!(
            model.starts_with("spice:"),
            spice_model(model).is_some(),
            "{}: {model}",
            p.part.id
        );
    }
}
