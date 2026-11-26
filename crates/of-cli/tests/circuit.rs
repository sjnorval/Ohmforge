//! End-to-end tests of `ohmforge circuit check` and `ohmforge circuit fmt`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn ohmforge() -> Command {
    Command::new(env!("CARGO_BIN_EXE_ohmforge"))
}

fn parts() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

fn temp_dir(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("ohmforge-circuit-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

fn circuit(args: &[&str], files: &[&Path]) -> Output {
    ohmforge()
        .arg("circuit")
        .args(args)
        .arg("--parts")
        .arg(parts())
        .args(files)
        .output()
        .unwrap()
}

fn text(o: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&o.stdout),
        String::from_utf8_lossy(&o.stderr)
    )
}

/// SHA-256 of b"blink".
const BLINK_SHA: &str = "e14a51fded1fb9b43de815d3832da099053b583f1d2c01d0495a70f118df426c";

fn valid(sha: &str) -> String {
    format!(
        "format = 1\n\n[circuit]\nname = \"LED\"\nground = \"gnd\"\n\n\
[[component]]\nref = \"U1\"\npart = \"espressif.esp32-c3\"\n\n\
[component.firmware]\napp = \"fw/app.elf\"\napp_sha256 = \"{sha}\"\nboot = \"direct\"\n\n\
[[component]]\nref = \"R1\"\npart = \"generic.resistor\"\nparams = {{ resistance = 330 }}\n\n\
[[component]]\nref = \"D1\"\npart = \"generic.led-red-0603\"\n\n\
[[net]]\nname = \"gpio8\"\nkind = \"signal\"\npins = [\"U1.GPIO8\", \"R1.1\"]\n\n\
[[net]]\nname = \"led_a\"\nkind = \"signal\"\npins = [\"R1.2\", \"D1.A\"]\n\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = [\"U1.GND\", \"D1.K\"]\n"
    )
}

fn project(name: &str, firmware: &[u8], sha: &str) -> (PathBuf, PathBuf) {
    let d = temp_dir(name);
    std::fs::create_dir_all(d.join("fw")).unwrap();
    std::fs::write(d.join("fw/app.elf"), firmware).unwrap();
    let f = d.join("led.circuit.toml");
    std::fs::write(&f, valid(sha)).unwrap();
    (d, f)
}

#[test]
fn check_accepts_a_valid_circuit() {
    let (d, f) = project("ok", b"blink", BLINK_SHA);
    let out = circuit(&["check"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(String::from_utf8_lossy(&out.stdout).contains("ok  "), "{}", text(&out));
    // Already canonical.
    let out = circuit(&["fmt", "--check"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn check_reports_path_line_and_column() {
    let d = temp_dir("unknown-field");
    let f = d.join("bad.circuit.toml");
    std::fs::write(&f, "format = 1\n[circuit]\nname = \"x\"\ncolour = \"red\"\n").unwrap();
    let out = circuit(&["check"], &[&f]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(&format!("{}:4:1: ", f.display())), "{err}");
    assert!(err.contains("colour"), "{err}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn check_lists_every_problem_including_library_ones() {
    let d = temp_dir("many");
    let f = d.join("many.circuit.toml");
    let src = valid(BLINK_SHA)
        .replace("generic.resistor", "acme.flux")
        .replace("\"D1.A\"", "\"D1.X\"")
        .replace("[\"U1.GND\", \"D1.K\"]", "[\"U1.GND\", \"D1.K\", \"R9.1\"]");
    std::fs::write(&f, src).unwrap();
    std::fs::create_dir_all(d.join("fw")).unwrap();
    std::fs::write(d.join("fw/app.elf"), b"blink").unwrap();
    let out = circuit(&["check"], &[&f]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains("unknown ref 'R9'"), "{err}");
    assert!(err.contains("unknown part 'acme.flux'"), "{err}");
    assert!(err.contains("has no pin 'X'"), "{err}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn firmware_hash_mismatch_is_an_error_until_updated() {
    let (d, f) = project("hash", b"changed firmware", BLINK_SHA);
    let out = circuit(&["check"], &[&f]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(err.contains(&format!("{}:13:14: ", f.display())), "{err}");
    assert!(err.contains("--update-hashes"), "{err}");

    let out = circuit(&["fmt", "--check", "--update-hashes"], &[&f]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(text(&out).contains("would reformat"), "{}", text(&out));

    let out = circuit(&["fmt", "--update-hashes"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    let out = circuit(&["check"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn update_hashes_fills_in_a_placeholder() {
    let (d, f) = project("placeholder", b"blink", "<64 hex>");
    assert_eq!(circuit(&["check"], &[&f]).status.code(), Some(1));
    let out = circuit(&["fmt", "--update-hashes"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    assert!(std::fs::read_to_string(&f).unwrap().contains(BLINK_SHA));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn fmt_check_flags_and_fmt_rewrites_non_canonical_files() {
    let (d, f) = project("fmt", b"blink", BLINK_SHA);
    let messy = valid(BLINK_SHA)
        .replace("\n\n", "\n")
        .replace("\"D1.K\"", "\"d1.k\"")
        .replace("kind = \"signal\"\n", "");
    std::fs::write(&f, &messy).unwrap();
    let out = circuit(&["fmt", "--check"], &[&f]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    assert!(
        text(&out).contains(&format!("would reformat {}", f.display())),
        "{}",
        text(&out)
    );
    assert_eq!(std::fs::read_to_string(&f).unwrap(), messy, "--check must not write");

    let out = circuit(&["fmt"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    assert_eq!(std::fs::read_to_string(&f).unwrap(), valid(BLINK_SHA));
    let out = circuit(&["fmt", "--check"], &[&f]);
    assert!(out.status.success(), "{}", text(&out));
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn directories_are_searched_for_circuit_files() {
    let (d, f) = project("dir", b"blink", BLINK_SHA);
    std::fs::write(d.join("broken.circuit.toml"), "format = 1\n").unwrap();
    std::fs::write(d.join("notes.toml"), "not a circuit").unwrap();
    let out = circuit(&["check"], &[&d]);
    assert_eq!(out.status.code(), Some(1), "{}", text(&out));
    let all = text(&out);
    assert!(all.contains(&format!("ok  {}", f.display())), "{all}");
    assert!(all.contains("broken.circuit.toml:"), "{all}");
    assert!(!all.contains("notes.toml"), "{all}");
    std::fs::remove_dir_all(&d).ok();
}

#[test]
fn usage_errors_exit_with_2() {
    let code = |args: &[&str]| ohmforge().args(args).output().unwrap().status.code();
    assert_eq!(code(&["circuit"]), Some(2));
    assert_eq!(code(&["circuit", "check"]), Some(2));
    assert_eq!(code(&["circuit", "lint", "x"]), Some(2));
    assert_eq!(code(&["circuit", "fmt", "--bogus", "x"]), Some(2));
    assert_eq!(code(&["circuit", "check", "--check", "x"]), Some(2));
}

#[test]
fn check_reports_a_signal_that_does_not_resolve_with_its_line_and_column() {
    // In `target/tmp`, so the demo's relative firmware path can reach `circuits/demos`.
    let file = Path::new(env!("CARGO_TARGET_TMPDIR")).join(format!("bad-signal-{}.circuit.toml", std::process::id()));
    let demo =
        std::fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../circuits/demos/blink.circuit.toml"))
            .unwrap()
            .replace("\r\n", "\n");
    // Firmware paths are relative to the file: point them back at the demos.
    let text = demo.replace("app = \"", "app = \"../../circuits/demos/");
    let text = text.replace("signal = \"i(R1)\"", "signal = \"v(nowhere)\"");
    std::fs::write(&file, &text).unwrap();
    let o = std::process::Command::new(env!("CARGO_BIN_EXE_ohmforge"))
        .current_dir(Path::new(env!("CARGO_MANIFEST_DIR")).join("../.."))
        .args(["circuit", "check", file.to_str().unwrap()])
        .output()
        .unwrap();
    let err = String::from_utf8_lossy(&o.stderr);
    assert!(!o.status.success(), "{err}");
    let line = text.lines().position(|l| l.contains("v(nowhere)")).unwrap() + 1;
    assert!(
        err.contains(&format!(":{line}:10: probe 'LED current': no net 'nowhere'")),
        "{err}"
    );
    std::fs::remove_file(&file).ok();
}
