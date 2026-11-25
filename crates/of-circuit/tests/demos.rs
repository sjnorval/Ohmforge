//! The shipped demo circuits (`circuits/demos`, features `demos` and `fs`).
#![cfg(all(feature = "demos", feature = "fs"))]

use std::path::Path;

use of_circuit::demos::{DEMOS, FIRMWARE, demo, resolver};
use of_circuit::fs::{FsResolver, load_parts};
use of_circuit::library::{check_firmware, check_parts, normalize};
use of_circuit::{BootKind, parse, save};

fn repo() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/../.."))
}

#[test]
fn there_are_five_demos_and_the_embedded_text_is_the_file() {
    let names: Vec<_> = DEMOS.iter().map(|d| d.name).collect();
    assert_eq!(names, ["blink", "dimmer", "fade", "button", "button-irq"]);
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
        normalize(&mut f, Some(&lib));
        assert_eq!(save(&f), d.text.replace("\r\n", "\n"), "{} is not canonical", d.path);
        assert_eq!(f.circuit.ground.get_ref(), "gnd", "{}", d.path);
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
        let u1 = f.components.iter().find(|c| c.reference() == "U1").unwrap();
        assert_eq!(
            u1.params.get("quiescent_a"),
            // ESP32-C3 datasheet v2.5, Table 5-8, 160 MHz: 23 mA with the CPU
            // running, 16 mA for button-irq, which sleeps in `wfi`.
            Some(&of_circuit::ParamValue::Number(if d.name == "button-irq" {
                0.016
            } else {
                0.023
            })),
            "{}",
            d.path
        );
        let fw = u1.firmware.as_ref().unwrap();
        let want = if d.name == "blink" {
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
