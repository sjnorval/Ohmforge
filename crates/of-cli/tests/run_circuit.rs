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

/// `(header, rows)` of a CSV file.
fn csv(path: &Path) -> (String, Vec<Vec<f64>>) {
    let data = std::fs::read_to_string(path).unwrap();
    let mut lines = data.lines();
    let header = lines.next().unwrap().to_owned();
    let rows = lines
        .map(|l| l.split(',').map(|v| v.parse().unwrap()).collect())
        .collect();
    (header, rows)
}

#[test]
fn log_writes_the_probes_as_csv_and_vcd_equal_to_the_solver_trace() {
    let dir = std::env::temp_dir().join(format!("ohmforge-run-log-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let all = dir.join("blink-all.csv");
    let log = dir.join("blink-log.csv");
    let vcd = dir.join("blink-log.vcd");
    for target in [&log, &vcd] {
        let o = run(&[
            "circuits/demos/blink.circuit.toml",
            "--until",
            "1ms",
            "--step",
            "50us",
            "--out",
            all.to_str().unwrap(),
            "--log",
            target.to_str().unwrap(),
        ]);
        assert!(o.status.success(), "{}", stderr(&o));
    }
    let (header, rows) = csv(&log);
    // The instruments (XMM1, XSC1) read the `LED current` probe, so it is the one column.
    assert_eq!(header, "time,LED current");
    let (all_header, all_rows) = csv(&all);
    assert_eq!(all_header.split(',').next_back(), Some("i(R1)"));
    assert_eq!(rows.len(), all_rows.len());
    for (r, a) in rows.iter().zip(&all_rows) {
        assert_eq!(r[0].to_bits(), a[0].to_bits(), "same sample times");
        let (led, old) = (r[1], *a.last().unwrap());
        // PARITY: max(1 nA, 1 ppm).
        assert!((led - old).abs() <= 1e-9_f64.max(1e-6 * old.abs()), "{led} vs {old}");
    }
    assert!(
        rows.iter().any(|r| r[1] > 1e-3),
        "the LED is lit at some point: {rows:?}"
    );
    let v = std::fs::read_to_string(&vcd).unwrap();
    assert!(v.contains("$var real 64") && v.contains("LED"), "{v}");
    std::fs::remove_dir_all(&dir).ok();
}

#[test]
fn log_needs_a_csv_or_vcd_file() {
    let o = run(&["circuits/demos/blink.circuit.toml", "--log", "out.txt"]);
    assert!(!o.status.success());
    assert!(stderr(&o).contains(".csv or .vcd"), "{}", stderr(&o));
}

/// A Fluke 87V on a 10 kOhm/10 kOhm divider at 3.3 V (0.1 Ohm PSU) loads it with 10 MOhm (E5f).
#[test]
fn a_real_meter_loads_the_logged_circuit() {
    let dir = std::env::temp_dir().join(format!("ohmforge-run-e5f-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("divider.circuit.toml");
    std::fs::write(
        &file,
        r#"format = 1

[circuit]
name = "Divider"

[[component]]
ref = "PSU"
part = "bench.psu"
params = { volts = 3.3 }

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 10000 }

[[component]]
ref = "R2"
part = "generic.resistor"
params = { resistance = 10000 }

[[net]]
name = "vdd"
kind = "power"
pins = ["PSU.+", "R1.1"]

[[net]]
name = "mid"
pins = ["R1.2", "R2.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["R2.2", "PSU.-"]

[[instrument]]
ref = "XMM1"
kind = "multimeter"
profile = "fluke.87v"
mode = "dc_v"
probes = ["v(mid)"]
"#,
    )
    .unwrap();
    let csv = dir.join("divider.csv");
    let o = run(&[
        file.to_str().unwrap(),
        "--until",
        "1ms",
        "--step",
        "500us",
        "--log",
        csv.to_str().unwrap(),
    ]);
    assert!(o.status.success(), "{}", stderr(&o));
    let data = std::fs::read_to_string(&csv).unwrap();
    let mut lines = data.lines();
    assert_eq!(lines.next(), Some("time,v(mid)"));
    let last: Vec<f64> = lines
        .next_back()
        .unwrap()
        .split(',')
        .map(|v| v.parse().unwrap())
        .collect();
    let low = 10e3 * 10e6 / (10e3 + 10e6);
    let want = 3.3 * low / (10e3 + 0.1 + low);
    assert!((last[1] - want).abs() <= 1e-6 * want, "{last:?} vs {want}");
}

/// A Saleae Logic 8 on a 9 kOhm/11 kOhm divider pin logs `d(R2.1)` as a 1-bit wire (E5g).
#[test]
fn a_logic_analyser_logs_digital_wires() {
    let dir = std::env::temp_dir().join(format!("ohmforge-run-e5g-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("logic.circuit.toml");
    std::fs::write(
        &file,
        r#"format = 1

[circuit]
name = "Logic"

[[component]]
ref = "PSU"
part = "bench.psu"
params = { volts = 3.3 }

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 9000 }

[[component]]
ref = "R2"
part = "generic.resistor"
params = { resistance = 11000 }

[[net]]
name = "vdd"
kind = "power"
pins = ["PSU.+", "R1.1"]

[[net]]
name = "mid"
pins = ["R1.2", "R2.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["R2.2", "PSU.-"]

[[instrument]]
ref = "XLA1"
kind = "logic"
profile = "saleae.logic8"
probes = ["d(R2.1)", "d(PSU.-)"]
"#,
    )
    .unwrap();
    for (name, check) in [("logic.csv", false), ("logic.vcd", true)] {
        let out = dir.join(name);
        let o = run(&[
            file.to_str().unwrap(),
            "--until",
            "1ms",
            "--step",
            "250us",
            "--log",
            out.to_str().unwrap(),
        ]);
        assert!(o.status.success(), "{}", stderr(&o));
        let data = std::fs::read_to_string(&out).unwrap();
        if check {
            assert!(data.contains("$var wire 1 ! d_R2_1 $end"), "{data}");
            assert!(data.contains("\n1!\n") && data.contains("\n0\"\n"), "{data}");
        } else {
            let mut lines = data.lines();
            assert_eq!(lines.next(), Some("time,d(R2.1),d(PSU.-)"));
            for l in lines {
                let v: Vec<f64> = l.split(',').map(|x| x.parse().unwrap()).collect();
                assert_eq!((v[1], v[2]), (1.0, 0.0), "{l}");
            }
        }
    }
}

#[test]
fn a_uart_console_decodes_its_tx_pin() {
    // An idle-high line (the divider's mid point) decodes to no bytes and no
    // framing errors; the console line is printed under --uart.
    let dir = std::env::temp_dir().join(format!("ohmforge-run-uart-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join("uart.circuit.toml");
    std::fs::write(
        &file,
        r#"format = 1

[circuit]
name = "Uart"

[[component]]
ref = "PSU"
part = "bench.psu"
params = { volts = 3.3 }

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 9000 }

[[component]]
ref = "R2"
part = "generic.resistor"
params = { resistance = 11000 }

[[net]]
name = "vdd"
kind = "power"
pins = ["PSU.+", "R1.1"]

[[net]]
name = "mid"
pins = ["R1.2", "R2.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["R2.2", "PSU.-"]

[[instrument]]
ref = "XU1"
kind = "uart"
probes = ["d(R2.1)"]
params = { baud = 115200 }
"#,
    )
    .unwrap();
    let o = run(&[file.to_str().unwrap(), "--until", "200us", "--uart"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    assert!(out.lines().any(|l| l == "XU1 (d(R2.1) at 115200 baud, 8N1):"), "{out}");
    assert!(!stderr(&o).contains("framing"), "{}", stderr(&o));
}

#[test]
fn the_uart_console_on_u0txd_matches_the_chips_serial_output() {
    // The blink demo with GPIO21 (U0TXD) wired to a 100 kΩ pull-down and
    // a UART console on it at the reset baud: XTAL 40 MHz / (SCLK_DIV_NUM 1
    // + 1) / CLKDIV 694 = 28 818 Bd (ESP32-C3 TRM v1.4, Registers 26.18 and
    // 26.7); the bare-metal blinky leaves UART0 unconfigured.
    // Firmware paths are relative to the circuit file, so the file goes
    // under the workspace (cargo's per-test scratch directory).
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../..")
        .canonicalize()
        .unwrap();
    let dir = std::path::Path::new(env!("CARGO_TARGET_TMPDIR")).join("u0txd");
    std::fs::create_dir_all(&dir).unwrap();
    let Ok(rel) = dir
        .canonicalize()
        .unwrap()
        .strip_prefix(&root)
        .map(|r| r.components().count())
    else {
        eprintln!("skipped: the target directory is outside the workspace");
        return;
    };
    let elf = format!("{}firmware/esp32c3-blinky/blinky.elf", "../".repeat(rel));
    let demo = std::fs::read_to_string(root.join("circuits/demos/blink.circuit.toml")).unwrap();
    let text = demo
        .replace("\"../../firmware/esp32c3-blinky/blinky.elf\"", &format!("\"{elf}\""))
        .replace(
            "pins = [\"U1.GND\", \"D1.K\", \"PSU.-\", \"GND.1\"]",
            "pins = [\"U1.GND\", \"D1.K\", \"PSU.-\", \"GND.1\", \"RTX.2\"]",
        )
        + r#"
[[component]]
ref = "RTX"
part = "generic.resistor"
params = { resistance = 100000 }

[[net]]
name = "tx"
pins = ["U1.GPIO21", "RTX.1"]

[[instrument]]
ref = "XU1"
kind = "uart"
probes = ["d(U1.GPIO21)"]
params = { baud = 28818 }
"#;
    let file = dir.join("blink-tx.circuit.toml");
    std::fs::write(&file, text).unwrap();
    // 16 frames of 10 bits at 28.8 kBd: 5.6 ms.
    let o = run(&[file.to_str().unwrap(), "--until", "7ms", "--step", "1ms", "--uart"]);
    assert!(o.status.success(), "{}", stderr(&o));
    let out = String::from_utf8_lossy(&o.stdout);
    let header = "XU1 (d(U1.GPIO21) at 28818 baud, 8N1):\n";
    let rest = out
        .split_once(header)
        .map(|(_, r)| r)
        .unwrap_or_else(|| panic!("{out}"));
    // The console's text, then the chip's own UART0 log: the same bytes.
    assert!(rest.contains("ohmforge blinky"), "{rest:?}");
    let (decoded, chip) = rest.split_at(rest.len() / 2);
    assert_eq!(decoded, chip, "{rest:?}");
    assert!(!stderr(&o).contains("framing"), "{}", stderr(&o));
}
