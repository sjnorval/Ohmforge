//! E2 acceptance: engine-built LED + resistor + PSU matches the bridge, KCL holds.

use std::collections::BTreeMap;
use std::sync::Arc;

use of_analog::analysis;
use of_components::analog::{Board, Env, Library, Placement, build};
use of_components::{BenchPsu, LibraryComponent, SupplySettings};
use of_engine::{EngineBuilder, OpView};
use of_parts::Corner;

fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

fn load_lib() -> Arc<Library> {
    Arc::new(Library::load_dir(&parts_dir()).expect("parts library"))
}

#[test]
fn engine_led_resistor_psu_matches_bridge() {
    let lib = load_lib();

    let board = Board {
        placements: vec![
            Placement::new("R1", "generic.resistor", &[("1", "vdd"), ("2", "anode")]).with("resistance", 330.0),
            Placement::new("D1", "generic.led-red-0603", &[("A", "anode"), ("K", "gnd")]),
        ],
        corner: Corner::Typical,
        env: Env::default(),
    };
    let mut built = build(&lib, &board).expect("bridge build");
    // Bridge board has no PSU; add Thevenin like the component does.
    let vdd = built.circuit.node("vdd");
    let gnd = built.circuit.find_node("gnd").unwrap();
    built
        .circuit
        .add(of_analog::devices::PinDriver::new("psu1", vdd, gnd, 3.3, 0.1))
        .unwrap();
    let op_bridge = analysis::operating_point(&mut built.circuit, &analysis::Options::default()).unwrap();
    let v_anode_b = op_bridge.get("v(anode)").unwrap();
    let i_psu_b = op_bridge.get("i(psu1)").unwrap();

    let mut eng = EngineBuilder::new("gnd");
    eng.net("vdd").net("anode");
    let r1 = LibraryComponent::new(
        "R1",
        "generic.resistor",
        Arc::clone(&lib),
        BTreeMap::from([("resistance".into(), 330.0)]),
        Corner::Typical,
        Env::default(),
    )
    .unwrap();
    let d1 = LibraryComponent::new(
        "D1",
        "generic.led-red-0603",
        Arc::clone(&lib),
        BTreeMap::new(),
        Corner::Typical,
        Env::default(),
    )
    .unwrap();
    let psu = BenchPsu::new(
        "PSU1",
        SupplySettings {
            on: true,
            volts: 3.3,
            series_r: 0.1,
            current_limit: 0.5,
            rated: None,
        },
    );
    eng.add(
        Box::new(r1),
        &[("1", "vdd"), ("2", "anode")],
        &[("resistance", 330.0)],
        0,
    )
    .unwrap();
    eng.add(Box::new(d1), &[("A", "anode"), ("K", "gnd")], &[], 0).unwrap();
    eng.add(
        Box::new(psu),
        &[("+", "vdd"), ("-", "gnd")],
        &[("volts", 3.3), ("series_r", 0.1)],
        0,
    )
    .unwrap();
    let mut built_e = eng.elaborate().expect("engine elaborate");
    let op = built_e.operating_point().expect("op");
    let v_anode = op.get("v(anode)").unwrap();
    let i_psu = op.get("i(psu1)").unwrap();

    assert!(
        (v_anode - v_anode_b).abs() < 1e-6,
        "anode {v_anode} vs bridge {v_anode_b}"
    );
    assert!(
        (i_psu - i_psu_b).abs() < 1e-6,
        "psu current {i_psu} vs bridge {i_psu_b}"
    );

    built_e.check_kcl(&op, 1e-6).expect("KCL");
}

#[test]
fn psu_plus_minus_currents_balance() {
    let lib = load_lib();
    let mut eng = EngineBuilder::new("gnd");
    eng.net("vdd");
    let r1 = LibraryComponent::new(
        "R1",
        "generic.resistor",
        lib,
        BTreeMap::from([("resistance".into(), 1_000.0)]),
        Corner::Typical,
        Env::default(),
    )
    .unwrap();
    let psu = BenchPsu::new("PSU1", SupplySettings::default());
    eng.add(Box::new(psu), &[("+", "vdd"), ("-", "gnd")], &[], 0).unwrap();
    eng.add(
        Box::new(r1),
        &[("1", "vdd"), ("2", "gnd")],
        &[("resistance", 1_000.0)],
        0,
    )
    .unwrap();
    let mut built = eng.elaborate().unwrap();
    let op = built.operating_point().unwrap();
    let psu = built
        .components
        .iter()
        .find(|c| c.component.info().part_id == "bench.psu")
        .unwrap();
    let mut i = [0.0; 2];
    psu.component.terminal_currents(&OpView(&op), &mut i);
    assert!((i[0] + i[1]).abs() < 1e-9, "PSU terminals {i:?}");
    // About 3.3 mA into the 1 kΩ load from the + pin outward => current into + is negative.
    assert!(i[0] < 0.0, "current into + should be negative (sourcing), got {i:?}");
}

#[test]
fn floating_net_is_rejected() {
    let mut eng = EngineBuilder::new("gnd");
    eng.net("orphan");
    match eng.elaborate() {
        Err(err) => assert!(err.to_string().contains("orphan"), "{err}"),
        Ok(_) => panic!("expected floating-net error"),
    }
}

#[test]
fn unconnected_supply_pin_is_rejected() {
    let psu = BenchPsu::new("PSU1", SupplySettings::default());
    let mut eng = EngineBuilder::new("gnd");
    eng.net("vdd");
    // Only connect +; omit −.
    match eng.add(Box::new(psu), &[("+", "vdd")], &[], 0) {
        Err(err) => assert!(err.to_string().contains("not connected"), "{err}"),
        Ok(_) => panic!("expected unconnected supply pin error"),
    }
}
