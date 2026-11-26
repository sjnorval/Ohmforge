//! Real-world instrument loading (engine E5f, RFC 0004): the circuit is
//! re-elaborated with each instrument's input, so readings carry the meter's
//! and the probe's loading.
#![cfg(feature = "engine")]

use std::sync::Arc;

use of_circuit::build::build;
use of_circuit::embedded::EmbeddedResolver;
use of_circuit::library::PartsIndex;
use of_circuit::loads::{instrument_loads, with_loads};
use of_circuit::parse;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::SimTime;
use of_engine::Registry;
use of_instruments::profile::LoadElement;
use of_instruments::{Probeable, Signal};

/// `bench.psu`'s default series resistance (`series_r`).
const RS: f64 = 0.1;

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
    reg
}

/// Analog parity: max(1 nA, 1 ppm).
fn parity(actual: f64, expected: f64) {
    let tol = 1e-9_f64.max(1e-6 * expected.abs());
    assert!((actual - expected).abs() <= tol, "{actual} vs {expected} (tol {tol})");
}

/// 3.3 V across R1 (top) and R2 (bottom), mid point `mid`, plus `extra`.
fn divider(r1: f64, r2: f64, extra: &str) -> String {
    format!(
        r#"format = 1

[circuit]
name = "Divider"

[[component]]
ref = "PSU"
part = "bench.psu"
params = {{ volts = 3.3 }}

[[component]]
ref = "R1"
part = "generic.resistor"
params = {{ resistance = {r1} }}

[[component]]
ref = "R2"
part = "generic.resistor"
params = {{ resistance = {r2} }}

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
{extra}"#
    )
}

fn run(src: &str, signal: &str) -> f64 {
    let f = parse(src).unwrap();
    let mut b = build(src, &f, &parts(), &registry(), &EmbeddedResolver::new()).unwrap();
    b.session.run_until(SimTime::from_ms(1)).unwrap();
    b.session.sample(&signal.parse::<Signal>().unwrap()).unwrap()
}

fn par(a: f64, b: f64) -> f64 {
    a * b / (a + b)
}

const METER_V: &str = r#"
[[instrument]]
ref = "XMM1"
kind = "multimeter"
profile = "fluke.87v"
mode = "dc_v"
probes = ["v(mid)"]
"#;

#[test]
fn ideal_meter_does_not_load_the_divider() {
    let ideal = METER_V.replace("profile = \"fluke.87v\"\n", "");
    parity(run(&divider(10e3, 10e3, &ideal), "v(mid)"), 3.3 * 10e3 / (20e3 + RS));
}

#[test]
fn fluke_87v_input_loads_a_10k_divider_to_1_6492_v() {
    let v = run(&divider(10e3, 10e3, METER_V), "v(mid)");
    let low = par(10e3, 10e6);
    parity(v, 3.3 * low / (10e3 + RS + low));
    assert!((v - 1.6492).abs() < 5e-5, "{v}");
}

#[test]
fn ds1054z_probe_loads_with_1_megohm() {
    let scope = r#"
[[instrument]]
ref = "XSC1"
kind = "scope"
profile = "rigol.ds1054z"
probes = ["v(mid)"]
"#;
    let src = divider(100e3, 100e3, scope);
    let f = parse(&src).unwrap();
    let loads = instrument_loads(&f, &parts());
    assert_eq!(
        loads[0].element,
        LoadElement::Shunt {
            net: "mid".into(),
            reference: "gnd".into(),
            ohms: 1e6,
            farads: 15e-12
        }
    );
    let low = par(100e3, 1e6);
    parity(run(&src, "v(mid)"), 3.3 * low / (100e3 + RS + low));
}

#[test]
fn series_ammeter_re_elaborates_with_its_burden() {
    let meter = r#"
[[instrument]]
ref = "XMM1"
kind = "multimeter"
profile = "fluke.87v"
mode = "dc_a"
probes = ["i(R1)"]
"#;
    // 660 ohm would pass 5 mA; the 1.8 ohm mA burden drops 1.8 mV/mA.
    let src = divider(330.0, 330.0, meter);
    let f = parse(&src).unwrap();
    let loaded = with_loads(&f, &instrument_loads(&f, &parts())).unwrap();
    let net = loaded.nets.iter().find(|n| n.name() == "XMM1_n1").expect("burden net");
    let pins: Vec<&str> = net.pins.iter().map(|p| p.get_ref().as_str()).collect();
    assert_eq!(pins, ["R1.1", "XMM1_R1.2"]);
    let i = run(&src, "i(R1)");
    parity(i, 3.3 / (661.8 + RS));
    let drop = run(&src, "v(vdd,XMM1_n1)");
    parity(drop, 1.8 * 3.3 / (661.8 + RS));
    assert!((drop - 9e-3).abs() < 0.1e-3, "{drop}");
}

#[test]
fn micro_amp_input_has_a_100_ohm_burden() {
    let meter = r#"
[[instrument]]
ref = "XMM1"
kind = "multimeter"
profile = "fluke.87v"
mode = "dc_a"
probes = ["i(R1)"]
params = { input = "uA" }
"#;
    let src = divider(10e3, 10e3, meter);
    parity(run(&src, "i(R1)"), 3.3 / (20_100.0 + RS));
}

#[test]
fn real_profiles_are_checked_against_the_instrument_kind() {
    let bad = METER_V.replace("fluke.87v", "rigol.ds1054z");
    let src = divider(10e3, 10e3, &bad);
    let f = parse(&src).unwrap();
    let errs = of_circuit::library::check_signals(&src, &f, &parts());
    assert!(
        errs.iter().any(|e| e.to_string().contains("is a scope profile")),
        "{errs:?}"
    );
    let unknown = METER_V.replace("fluke.87v", "acme.x1");
    let src = divider(10e3, 10e3, &unknown);
    let f = parse(&src).unwrap();
    let errs = of_circuit::library::check_signals(&src, &f, &parts());
    assert!(
        errs.iter()
            .any(|e| e.to_string().contains("unknown instrument profile")),
        "{errs:?}"
    );
}
