//! End-to-end tests of `ohmforge run <file.circuit.toml>`.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

fn repo() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_ohmforge"))
        .current_dir(repo())
        .arg("run")
        .args(args)
        .output()
        .unwrap()
}

fn stderr(o: &Output) -> String {
    String::from_utf8_lossy(&o.stderr).into_owned()
}

#[test]
fn blink_prints_its_uart_banner() {
    let o = run(&["circuits/demos/blink.circuit.toml", "--until", "3ms", "--uart"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("ohmforge blinky"), "{out}");
}

#[test]
fn blink_writes_nets_and_probes_as_csv_and_vcd() {
    let dir = std::env::temp_dir().join(format!("ohmforge-run-circuit-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let csv = dir.join("blink.csv");
    let vcd = dir.join("blink.vcd");
    let o = run(&[
        "circuits/demos/blink.circuit.toml",
        "--until",
        "1ms",
        "--step",
        "50us",
        "--out",
        csv.to_str().unwrap(),
        "--vcd",
        vcd.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let data = std::fs::read_to_string(&csv).unwrap();
    let mut lines = data.lines();
    assert_eq!(lines.next(), Some("time,v(gpio8),v(led_a),v(vdd),i(R1)"));
    let rows: Vec<Vec<f64>> = lines
        .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
        .collect();
    assert_eq!(rows.len(), 21);
    assert!((rows[20][0] - 1e-3).abs() < 1e-12, "{:?}", rows[20]);
    for r in &rows {
        assert!((r[3] - 3.3).abs() < 0.01, "rail {r:?}");
        assert!(r.iter().all(|v| v.is_finite()), "{r:?}");
    }
    let lit = rows.iter().any(|r| r[4] > 1e-3);
    assert!(lit, "the LED never turned on in 1 ms");
    assert!(std::fs::read_to_string(&vcd).unwrap().contains("$enddefinitions"));
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn rom_demo_runs_or_says_the_rom_is_missing() {
    let o = run(&["circuits/demos/fade.circuit.toml", "--until", "1ms", "--uart"]);
    if !o.status.success() && stderr(&o).contains("not bundled") {
        assert!(
            std::env::var_os("OF_REQUIRE_ESP_ROM").is_none(),
            "OF_REQUIRE_ESP_ROM is set: {}",
            stderr(&o)
        );
        eprintln!("skipping: {}", stderr(&o));
        return;
    }
    assert!(o.status.success(), "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.contains("ohmforge esp-hal led fade"), "{out}");
}

#[test]
fn bad_options_and_files_fail() {
    let o = run(&["circuits/demos/blink.circuit.toml", "--until", "soon"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("not a time"), "{}", stderr(&o));
    let o = run(&["missing.circuit.toml"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains("missing.circuit.toml"), "{}", stderr(&o));
}
