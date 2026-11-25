//! Parsing the format = 1 sections.

use of_circuit::{BootKind, Dir, NetKind, ParamValue, SimCorner, parse};

const MINI: &str = r#"
format = 1
[circuit]
name = "mini"
ground = "gnd"

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 330 }

[[component]]
ref = "PSU"
part = "bench.psu"

[[net]]
name = "vdd"
kind = "power"
pins = ["PSU.+", "R1.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["PSU.-", "R1.2"]
"#;

#[test]
fn parses_a_minimal_circuit() {
    let f = parse(MINI).expect("parse");
    assert_eq!(*f.format.get_ref(), 1);
    assert_eq!(f.circuit.ground.get_ref(), "gnd");
    assert_eq!(f.components.len(), 2);
    assert_eq!(f.components[0].reference(), "R1");
    assert_eq!(f.components[0].params["resistance"], ParamValue::Number(330.0));
    assert_eq!(f.nets[1].kind, NetKind::Ground);
    assert_eq!(f.nets[0].pins().collect::<Vec<_>>(), ["PSU.+", "R1.1"]);
}

#[test]
fn ground_defaults_to_gnd_and_sim_to_the_blink_settings() {
    let f =
        parse("format = 1\n[circuit]\nname = \"x\"\n[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n").unwrap();
    assert_eq!(f.circuit.ground.get_ref(), "gnd");
    assert_eq!(f.sim.quantum_us, 50);
    assert_eq!(f.sim.max_step, 5e-6);
    assert_eq!(f.sim.corner, SimCorner::Typ);
    assert_eq!(f.sim.seed, 0);
    assert_eq!(f.sim.temperature_c, None);
}

#[test]
fn integer_and_float_params_are_numbers() {
    let src = "format = 1\n[circuit]\nname = \"x\"\n[[component]]\nref = \"R1\"\npart = \"generic.resistor\"\n\
params = { a = 1, b = 2.5, c = true, d = \"fast\", e = [\"GPIO9\"] }\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = [\"R1.1\"]\n";
    let f = parse(src).unwrap();
    let p = &f.components[0].params;
    assert_eq!(p["a"], ParamValue::Number(1.0));
    assert_eq!(p["b"], ParamValue::Number(2.5));
    assert_eq!(p["c"], ParamValue::Bool(true));
    assert_eq!(p["d"], ParamValue::Text("fast".into()));
    assert_eq!(p["e"], ParamValue::List(vec!["GPIO9".into()]));
    assert_eq!(p["a"].as_number(), Some(1.0));
    assert_eq!(p["c"].as_number(), Some(1.0));
    assert_eq!(p["d"].as_number(), None);
}

#[test]
fn parses_firmware_schematic_symbols_keepouts_and_probes() {
    let src = include_str!("fixtures/all_sections.circuit.toml");
    let f = parse(src).unwrap();
    assert_eq!(f.circuit.description.as_deref(), Some("Every section of format 1."));
    assert_eq!(f.sim.quantum_us, 25);
    assert_eq!(f.sim.corner, SimCorner::Max);
    let u1 = &f.components[0];
    let fw = u1.firmware.as_ref().unwrap();
    assert_eq!(fw.boot, BootKind::Rom);
    assert_eq!(fw.app.get_ref(), "fw/app.bin");
    assert_eq!(fw.ready_text.as_deref(), Some("ready\n"));
    let sch = u1.schematic.as_ref().unwrap();
    assert_eq!(sch.body, [0.0, 0.0, 102.0, 100.0]);
    assert_eq!(sch.pins[0].pin.get_ref(), "3V3");
    assert_eq!(sch.pins[0].id.as_deref(), Some("vdd"));
    assert_eq!(sch.pins[0].dir, Dir::W);
    let rv1 = &f.components[2];
    let wiper = &rv1.schematic.as_ref().unwrap().pins[1];
    assert_eq!(wiper.slide.as_ref().unwrap().param.get_ref(), "position");
    assert!(rv1.schematic.as_ref().unwrap().pins[2].stub);
    assert_eq!(f.symbols[0].kind.get_ref(), "ground");
    assert_eq!(f.schematic.keepouts[0].note.as_deref(), Some("U1 label"));
    assert_eq!(f.probes[0].signal, "i(R1)");
    assert_eq!(f.nets[0].waypoints, vec![[40.0, 10.5]]);
}
