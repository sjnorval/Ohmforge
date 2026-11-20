//! Placed parts to solver devices: pin mapping, parameters, corners and
//! errors.

mod common;

use of_analog::devices::{SourceValue, VoltageSource};
use of_analog::{Options, analysis};
use of_components::Corner;
use of_components::analog::{Board, BridgeError, Placement, build, to_spice};

fn led_board(corner: Corner) -> Board {
    Board {
        placements: vec![
            Placement::new("R1", "generic.resistor", &[("1", "vdd"), ("2", "a")]).with("resistance", 330.0),
            Placement::new("D1", "generic.led-red-0603", &[("A", "a"), ("K", "gnd")]),
        ],
        corner,
        ..Board::default()
    }
}

fn op_led_current(board: &Board) -> f64 {
    let lib = common::library();
    let mut b = build(&lib, board).unwrap();
    let vdd = b.circuit.node("vdd");
    b.circuit
        .add(VoltageSource::new("v1", vdd, 0, SourceValue::dc(3.3)))
        .unwrap();
    let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
    -op.get("i(v1)").unwrap()
}

#[test]
fn led_with_series_resistor_from_parts() {
    let i = op_led_current(&led_board(Corner::Typical));
    // About 2 V across the LED at a few mA: (3.3 - Vf) / 330.
    assert!((3.0e-3..5.0e-3).contains(&i), "LED current {i}");
    let lib = common::library();
    let mut b = build(&lib, &led_board(Corner::Typical)).unwrap();
    assert_eq!(b.instances["R1"], "r1");
    let vdd = b.circuit.node("vdd");
    b.circuit
        .add(VoltageSource::new("v1", vdd, 0, SourceValue::dc(3.3)))
        .unwrap();
    let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
    let vf = op.get("v(a)").unwrap();
    assert!(((3.3 - vf) / 330.0 - i).abs() < 1e-9);
    assert!(op.get("popt(d1)").unwrap() > 0.0, "LED probes survive the bridge");
}

#[test]
fn placement_override_wins_and_tolerance_corners_apply() {
    // The override has no tolerance of its own, so the part's 1 % applies.
    let typ = op_led_current(&led_board(Corner::Typical));
    let lo = op_led_current(&led_board(Corner::Min));
    let hi = op_led_current(&led_board(Corner::Max));
    assert!(lo > typ && typ > hi, "{lo} {typ} {hi}");
    // A 1 % lower resistor gives roughly 1 % more current.
    assert!(((lo - typ) / typ - 0.01).abs() < 2e-3, "{}", (lo - typ) / typ);
}

#[test]
fn subcircuit_part_instances_are_independent() {
    let lib = common::library();
    let mut board = Board::default();
    for (u, out) in [("U1", "a"), ("U2", "b")] {
        board.placements.push(Placement::new(
            u,
            "ams.ams1117-3.3",
            &[("VIN", "in"), ("VOUT", out), ("GND", "0")],
        ));
        board.placements.push(
            Placement::new(&format!("RL{u}"), "generic.resistor", &[("1", out), ("2", "0")]).with("resistance", 33.0),
        );
    }
    board.placements[0] = board.placements[0].clone().with("vout", 5.0);
    let mut b = build(&lib, &board).unwrap();
    assert_eq!(b.instances["U2"], "xu2");
    let vin = b.circuit.node("in");
    b.circuit
        .add(VoltageSource::new("vin", vin, 0, SourceValue::dc(7.0)))
        .unwrap();
    let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
    assert!((op.get("v(a)").unwrap() - 5.0).abs() < 0.03, "{:?}", op.get("v(a)"));
    assert!((op.get("v(b)").unwrap() - 3.3).abs() < 0.02, "{:?}", op.get("v(b)"));
    // Junction temperature rise = pass-transistor power x 90 K/W.
    for (u, out, vout) in [("xu1", "v(a)", 5.0), ("xu2", "v(b)", 3.3)] {
        let p = (7.0 - op.get(out).unwrap()) * vout / 33.0;
        let rise = op.get(&format!("v({u}.tj)")).unwrap(); // rise over ambient
        assert!((rise - 90.0 * p).abs() < 0.02 * 90.0 * p, "{u}: {rise} K for {p} W");
    }
}

#[test]
fn sensor_supply_load_and_unconnected_digital_pins() {
    let lib = common::library();
    let board = Board {
        placements: vec![Placement::new(
            "U3",
            "bosch.bme280",
            &[("VDD", "vdd"), ("GND", "gnd"), ("SDA", "sda")],
        )],
        ..Board::default()
    };
    let mut b = build(&lib, &board).unwrap();
    let vdd = b.circuit.node("vdd");
    b.circuit
        .add(VoltageSource::new("v1", vdd, 0, SourceValue::dc(3.3)))
        .unwrap();
    let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
    assert!((-op.get("i(v1)").unwrap() - 3.6e-6).abs() < 1e-9);
    // The peak-current corner.
    let hi = Board {
        corner: Corner::Max,
        ..board
    };
    let mut b = build(&lib, &hi).unwrap();
    let vdd = b.circuit.node("vdd");
    b.circuit
        .add(VoltageSource::new("v1", vdd, 0, SourceValue::dc(3.3)))
        .unwrap();
    let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
    assert!((-op.get("i(v1)").unwrap() - 714e-6).abs() < 1e-8);
}

#[test]
fn errors_name_the_placement() {
    let lib = common::library();
    let one = |p: Placement| {
        build(
            &lib,
            &Board {
                placements: vec![p],
                ..Board::default()
            },
        )
        .unwrap_err()
    };
    assert_eq!(
        one(Placement::new("R9", "no.such", &[])),
        BridgeError::UnknownPart("R9".into(), "no.such".into())
    );
    assert!(matches!(
        one(Placement::new("R9", "generic.resistor", &[("1", "a")])),
        BridgeError::Unconnected(r, _) if r == "R9"
    ));
    assert!(matches!(
        one(Placement::new(
            "R9",
            "generic.resistor",
            &[("1", "a"), ("2", "b"), ("7", "c")]
        )),
        BridgeError::UnknownPin(..)
    ));
    assert!(matches!(
        one(Placement::new("R9", "generic.resistor", &[("1", "a"), ("2", "b")]).with("resistance", -1.0)),
        BridgeError::Param(..)
    ));
    let dup = Board {
        placements: vec![
            Placement::new("R1", "generic.resistor", &[("1", "a"), ("2", "0")]),
            Placement::new("R1", "generic.resistor", &[("1", "b"), ("2", "0")]),
        ],
        ..Board::default()
    };
    assert_eq!(build(&lib, &dup).unwrap_err(), BridgeError::Duplicate("R1".into()));
}

#[test]
fn spice_export_round_trips() {
    let lib = common::library();
    let mut board = led_board(Corner::Typical);
    board.placements.push(Placement::new(
        "U1",
        "ams.ams1117-3.3",
        &[("VIN", "usb"), ("VOUT", "vdd"), ("GND", "0")],
    ));
    board.placements.push(Placement::new(
        "C1",
        "generic.capacitor-tantalum",
        &[("+", "vdd"), ("-", "0")],
    ));
    let deck = to_spice(&lib, &board).unwrap();
    assert!(deck.contains(".subckt ams1117"), "{deck}");
    let text = format!("export\n{deck}VUSB usb 0 5\n.op\n.end\n");
    let parsed = of_netlist::spice::parse_str(&text).unwrap();
    let mut from_deck = of_analog::netlist::build(&parsed).unwrap();
    let a = analysis::operating_point(&mut from_deck.circuit, &from_deck.options).unwrap();
    let mut b = build(&lib, &board).unwrap();
    let usb = b.circuit.node("usb");
    b.circuit
        .add(VoltageSource::new("vusb", usb, 0, SourceValue::dc(5.0)))
        .unwrap();
    let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
    for sig in ["v(vdd)", "v(a)", "i(vusb)", "v(xu1.tj)"] {
        let (x, y) = (a.get(sig).unwrap(), op.get(sig).unwrap());
        assert!((x - y).abs() <= 1e-9 * x.abs().max(1.0), "{sig}: deck {x}, bridge {y}");
    }
}

fn button_board(closed: Option<f64>) -> Board {
    let mut sw = Placement::new("SW1", "generic.pushbutton", &[("1", "btn"), ("2", "gnd")]);
    if let Some(c) = closed {
        sw = sw.with("closed", c);
    }
    Board {
        placements: vec![
            Placement::new("R1", "generic.resistor", &[("1", "vdd"), ("2", "btn")]).with("resistance", 10e3),
            sw,
        ],
        ..Board::default()
    }
}

#[test]
fn push_button_with_pull_up_is_a_switched_resistor() {
    let lib = common::library();
    let v_btn = |board: &Board| {
        let mut b = build(&lib, board).unwrap();
        let vdd = b.circuit.node("vdd");
        b.circuit
            .add(VoltageSource::new("v1", vdd, 0, SourceValue::dc(3.3)))
            .unwrap();
        let op = analysis::operating_point(&mut b.circuit, &Options::default()).unwrap();
        op.get("v(btn)").unwrap()
    };
    // Open (the default): 10 kΩ against 100 MΩ.
    let open = v_btn(&button_board(None));
    assert!((open - 3.3 * 100e6 / (100e6 + 10e3)).abs() < 1e-6, "{open}");
    assert_eq!(v_btn(&button_board(Some(0.0))), open);
    // Closed: 10 kΩ against 0.1 Ω.
    let closed = v_btn(&button_board(Some(1.0)));
    assert!(closed < 1e-4 && closed > 0.0, "{closed}");

    let b = build(&lib, &button_board(None)).unwrap();
    assert_eq!(b.instances["SW1"], "sw1");
    assert_eq!(
        b.switch_param("SW1", true).map(|p| (p.device, p.param, p.value)),
        Some(("sw1".into(), "r".into(), 0.1))
    );
    assert_eq!(b.switch_param("SW1", false).unwrap().value, 100e6);
    assert_eq!(b.switch_param("R1", true), None, "not a switch");
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);

    let deck = to_spice(&lib, &button_board(Some(1.0))).unwrap();
    assert!(deck.contains("Rsw1 btn gnd 1e-1"), "{deck}");
}
