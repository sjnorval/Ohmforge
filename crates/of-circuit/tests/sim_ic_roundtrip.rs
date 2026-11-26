//! `[sim] uic` and `ic` survive a save and reload (RFC 0003).

use of_circuit::{parse, same_content, save};

#[test]
fn uic_and_ic_survive_save_and_reload() {
    let src = r#"format = 1

[circuit]
name = "ic round trip"
ground = "gnd"

[sim]
max_step = 1e-5
uic = true
ic = { cap = 1.25, vcc = 5 }

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 1000 }

[[component]]
ref = "C1"
part = "generic.capacitor-mlcc"
params = { capacitance = 1e-7 }

[[net]]
name = "vcc"
pins = ["R1.1"]

[[net]]
name = "cap"
pins = ["R1.2", "C1.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["C1.2"]
"#;
    let a = parse(src).unwrap();
    assert!(a.sim.uic);
    assert_eq!(a.sim.ic.get("cap"), Some(&1.25));
    assert_eq!(a.sim.ic.get("vcc"), Some(&5.0));
    let text = save(&a);
    let b = parse(&text).unwrap_or_else(|e| panic!("reload: {e:?}\n{text}"));
    assert!(b.sim.uic, "uic lost:\n{text}");
    assert_eq!(b.sim.ic, a.sim.ic, "ic changed:\n{text}");
    assert!(same_content(&a, &b));
    // And saving again is stable.
    assert_eq!(save(&b), text);

    // Without uic nothing is written.
    let plain = src.replace("uic = true\nic = { cap = 1.25, vcc = 5 }\n", "");
    let c = parse(&plain).unwrap();
    let text = save(&c);
    assert!(!text.contains("uic") && !text.contains("ic ="), "{text}");
    let d = parse(&text).unwrap();
    assert!(!d.sim.uic && d.sim.ic.is_empty());
}
