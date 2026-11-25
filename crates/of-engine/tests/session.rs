//! E4b: registry and `Session` runtime (engine plan Tasks 9, 10 and 12).

use std::collections::BTreeMap;
use std::sync::Arc;

use of_component_api::ParamValue;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::SimTime;
use of_engine::{ComponentSpec, EngineBuilder, Registry, Session, SessionConfig};

fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

fn registry() -> Registry {
    let lib = Arc::new(Library::load_dir(&parts_dir()).expect("parts library"));
    let mut reg = Registry::new();
    register_builtin(&mut reg, lib, Corner::Typical, Env::default());
    reg
}

/// One placement for [`build`]: ref, part, model, params, pins (pin, net).
type Place<'a> = (
    &'a str,
    &'a str,
    &'a str,
    Vec<(&'a str, ParamValue)>,
    Vec<(&'a str, &'a str)>,
);

fn build(reg: &Registry, places: Vec<Place<'_>>, cfg: SessionConfig) -> Session {
    let mut eng = EngineBuilder::new("gnd");
    for (_, _, _, _, pins) in &places {
        for (_, net) in pins {
            eng.net(net);
        }
    }
    for (refdes, part, model, params, pins) in places {
        let params: BTreeMap<String, ParamValue> = params.into_iter().map(|(k, v)| (k.to_owned(), v)).collect();
        let pin_names: Vec<String> = pins.iter().map(|(p, _)| (*p).to_owned()).collect();
        let spec = ComponentSpec {
            refdes,
            part_id: part,
            params: &params,
            firmware: None,
            pins: &pin_names,
            seed: 0,
        };
        let numeric = spec.numeric_params();
        let c = reg.build(model, spec).expect("build");
        let num: Vec<(&str, f64)> = numeric.iter().map(|(k, v)| (k.as_str(), *v)).collect();
        eng.add(c, &pins, &num, 0).expect("add");
    }
    eng.elaborate().expect("elaborate").into_session(cfg).expect("session")
}

#[test]
fn unknown_model_names_the_refdes_and_the_model() {
    let reg = registry();
    let params = BTreeMap::new();
    let err = reg
        .build(
            "rust:flux-capacitor",
            ComponentSpec {
                refdes: "X1",
                part_id: "acme.flux",
                params: &params,
                firmware: None,
                pins: &[],
                seed: 0,
            },
        )
        .err()
        .expect("unknown model");
    let msg = err.to_string();
    assert!(msg.contains("X1") && msg.contains("rust:flux-capacitor"), "{msg}");
}

#[test]
fn builtins_cover_the_demo_models_with_part_pins() {
    let reg = registry();
    for (model, part, pins) in [
        ("rust:resistor", "generic.resistor", vec!["1", "2"]),
        ("rust:led", "generic.led-red-0603", vec!["A", "K"]),
        ("rust:potentiometer", "bourns.ptv09a-b103", vec!["1", "W", "3"]),
        ("rust:switch", "generic.pushbutton", vec!["1", "2"]),
        ("rust:bench-psu", "bench.psu", vec!["+", "-"]),
    ] {
        let params = BTreeMap::new();
        let c = reg
            .build(
                model,
                ComponentSpec {
                    refdes: "X1",
                    part_id: part,
                    params: &params,
                    firmware: None,
                    pins: &[],
                    seed: 0,
                },
            )
            .unwrap_or_else(|e| panic!("{model}: {e}"));
        let got: Vec<&str> = c.info().pins.iter().map(|p| p.name.as_str()).collect();
        assert_eq!(got, pins, "{model}");
    }
}

fn led_board(reg: &Registry, psu: Vec<(&'static str, ParamValue)>, r: f64) -> Session {
    build(
        reg,
        vec![
            (
                "R1",
                "generic.resistor",
                "rust:resistor",
                vec![("resistance", r.into())],
                vec![("1", "vdd"), ("2", "anode")],
            ),
            (
                "D1",
                "generic.led-red-0603",
                "rust:led",
                vec![],
                vec![("A", "anode"), ("K", "gnd")],
            ),
            (
                "PSU",
                "bench.psu",
                "rust:bench-psu",
                psu,
                vec![("+", "vdd"), ("-", "gnd")],
            ),
        ],
        SessionConfig::default(),
    )
}

#[test]
fn led_session_holds_the_operating_point() {
    let reg = registry();
    let mut s = led_board(&reg, vec![], 330.0);
    let v0 = s.value("v(anode)").expect("anode");
    assert_eq!(s.origin(), SimTime::ZERO);
    s.run_until(SimTime::from_ms(1)).expect("run");
    assert_eq!(s.now(), SimTime::from_ms(1));
    let v1 = s.value("v(anode)").expect("anode");
    assert!((v1 - v0).abs() < 1e-9, "{v0} -> {v1}");
    assert!((1.6..2.2).contains(&v1), "LED forward voltage {v1}");
}

#[test]
fn psu_goes_to_constant_current_and_droops_like_the_desktop_supply() {
    // Same numbers as of-ui supply.rs `current_limit_clamps_and_droops_the_rail`:
    // 330 ohm at 3.3 V would draw 10 mA; a 1 mA limit droops the rail.
    let reg = registry();
    let mut s = build(
        &reg,
        vec![
            (
                "R1",
                "generic.resistor",
                "rust:resistor",
                vec![("resistance", 330.0.into())],
                vec![("1", "vdd"), ("2", "gnd")],
            ),
            (
                "PSU",
                "bench.psu",
                "rust:bench-psu",
                vec![("current_limit", 1e-3.into())],
                vec![("+", "vdd"), ("-", "gnd")],
            ),
        ],
        SessionConfig::default(),
    );
    s.run_until(SimTime::from_us(50)).unwrap();
    s.commit().unwrap();
    s.run_until(SimTime::from_us(100)).unwrap();
    s.commit().unwrap();
    let r: BTreeMap<String, f64> = s.readings("PSU").into_iter().collect();
    assert_eq!(r["mode"], 2.0, "{r:?}");
    assert!((r["regulated_amps"] - 1e-3).abs() < 5e-5, "{r:?}");
    let v = s.value("v(vdd)").unwrap();
    assert!((v - 0.33).abs() < 0.05, "expected droop, got {v}");
}

/// A new set point is regulated against a rail solved at that set point:
/// committing before the next step (the rail still at the old 3.3 V) must not
/// read the step as an overload and switch to constant current, which drives
/// the limit into the load (tens of volts on a 10 kΩ divider).
#[test]
fn a_set_point_change_before_the_next_step_stays_in_constant_voltage() {
    let reg = registry();
    let mut s = build(
        &reg,
        vec![
            (
                "RV1",
                "bourns.ptv09a-b103",
                "rust:potentiometer",
                vec![("position", 0.5.into())],
                vec![("1", "gnd"), ("W", "wiper"), ("3", "vdd")],
            ),
            (
                "R1",
                "generic.resistor",
                "rust:resistor",
                vec![("resistance", 1e6.into())],
                vec![("1", "wiper"), ("2", "gnd")],
            ),
            (
                "PSU",
                "bench.psu",
                "rust:bench-psu",
                vec![],
                vec![("+", "vdd"), ("-", "gnd")],
            ),
        ],
        SessionConfig::default(),
    );
    s.set_param("PSU", "volts", 5.0).unwrap();
    s.commit().unwrap();
    let r: BTreeMap<String, f64> = s.readings("PSU").into_iter().collect();
    assert_eq!(r["mode"], 1.0, "a set point step is not an overload: {r:?}");
    for us in 1..=20 {
        s.run_until(SimTime::from_us(us)).unwrap();
        s.commit().unwrap();
        let v = s.value("v(vdd)").unwrap();
        let w = s.value("v(wiper)").unwrap();
        assert!(v <= 5.0 + 1e-6, "rail {v} V above the 5 V set point at {us} us");
        assert!(w <= v + 1e-9, "wiper {w} V above the rail {v} V at {us} us");
    }
    assert!((s.value("v(vdd)").unwrap() - 5.0).abs() < 0.01);
}

#[test]
fn pot_position_moves_the_wiper() {
    let reg = registry();
    let mut s = build(
        &reg,
        vec![
            (
                "RV1",
                "bourns.ptv09a-b103",
                "rust:potentiometer",
                vec![("position", 0.5.into())],
                vec![("1", "gnd"), ("W", "wiper"), ("3", "vdd")],
            ),
            (
                "R1",
                "generic.resistor",
                "rust:resistor",
                vec![("resistance", 1e6.into())],
                vec![("1", "wiper"), ("2", "gnd")],
            ),
            (
                "PSU",
                "bench.psu",
                "rust:bench-psu",
                vec![],
                vec![("+", "vdd"), ("-", "gnd")],
            ),
        ],
        SessionConfig::default(),
    );
    let wiper = |s: &mut Session, pos: f64, t_us: u64| {
        s.set_param("RV1", "position", pos).unwrap();
        s.run_until(SimTime::from_us(t_us)).unwrap();
        s.value("v(wiper)").unwrap() / s.value("v(vdd)").unwrap()
    };
    let w2 = wiper(&mut s, 0.2, 10);
    let w9 = wiper(&mut s, 0.9, 20);
    // 1 MΩ load on the wiper moves the ratio by well under 1 %.
    assert!((w2 - 0.2).abs() < 0.01, "{w2}");
    assert!((w9 - 0.9).abs() < 0.01, "{w9}");

    // Terminal currents in pin order (1, W, 3): KCL holds, and the 10 kΩ
    // track carries about VDD / 10 kΩ from pin 3 down to pin 1.
    let i = s.terminal_currents("RV1").expect("RV1");
    assert_eq!(i.len(), 3);
    assert!(i.iter().sum::<f64>().abs() < 1e-12, "{i:?}");
    let track = s.value("v(vdd)").unwrap() / 10e3;
    assert!((i[2] - track).abs() < 0.02 * track, "{i:?} vs {track}");
    assert!((i[0] + track).abs() < 0.02 * track, "{i:?} vs {track}");
    assert!(s.terminal_currents("RV9").is_none());
}

#[test]
fn switch_bounces_on_timers_then_settles() {
    let reg = registry();
    let mut s = build(
        &reg,
        vec![
            (
                "SW1",
                "generic.pushbutton",
                "rust:switch",
                vec![],
                vec![("1", "vdd"), ("2", "out")],
            ),
            (
                "R1",
                "generic.resistor",
                "rust:resistor",
                vec![("resistance", 1e3.into())],
                vec![("1", "out"), ("2", "gnd")],
            ),
            (
                "PSU",
                "bench.psu",
                "rust:bench-psu",
                vec![],
                vec![("+", "vdd"), ("-", "gnd")],
            ),
        ],
        SessionConfig::default(),
    );
    assert!(s.value("v(out)").unwrap() < 1e-3);
    s.set_param("SW1", "closed", 1.0).unwrap();
    // Press bounce: closed at 0, open at 30 us, closed at 60, open at 100, closed at 150.
    let times: Vec<SimTime> = s.pending_timers().map(|(t, _, _)| t).collect();
    assert_eq!(
        times,
        [30, 60, 100, 150].map(SimTime::from_us).to_vec(),
        "bounce timers at the bounce_script times"
    );
    let mut seen = Vec::new();
    for t in [10, 40, 80, 120, 200] {
        s.run_until(SimTime::from_us(t)).unwrap();
        seen.push(s.value("v(out)").unwrap() > 1.0);
    }
    assert_eq!(seen, [true, false, true, false, true]);

    // A bounce seed from the host jitters the script (same edge count).
    let offsets = |s: &Session, at: u64| -> Vec<u64> {
        s.pending_timers()
            .map(|(t, _, _)| t.0 - SimTime::from_us(at).0)
            .collect()
    };
    s.run_until(SimTime::from_us(400)).unwrap();
    s.set_param("SW1", "bounce_seed", 7.0).unwrap();
    s.set_param("SW1", "closed", 0.0).unwrap();
    let jittered = offsets(&s, 400);
    s.run_until(SimTime::from_us(800)).unwrap();
    s.set_param("SW1", "closed", 1.0).unwrap();
    s.run_until(SimTime::from_us(1200)).unwrap();
    s.set_param("SW1", "bounce_seed", 0.0).unwrap();
    s.set_param("SW1", "closed", 0.0).unwrap();
    let plain = offsets(&s, 1200);
    assert_eq!(jittered.len(), plain.len(), "{jittered:?} vs {plain:?}");
    assert_ne!(jittered, plain, "seed 7 must move the bounce edges");
    s.run_until(SimTime::from_us(1600)).unwrap();
    s.set_param("SW1", "closed", 1.0).unwrap();
    s.run_until(SimTime::from_us(2000)).unwrap();

    // With bounce off a release is one clean edge.
    s.set_param("SW1", "bounce", 0.0).unwrap();
    s.set_param("SW1", "closed", 0.0).unwrap();
    assert_eq!(s.pending_timers().count(), 0);
    s.run_until(SimTime::from_us(2010)).unwrap();
    assert!(s.value("v(out)").unwrap() < 1e-3);
}

#[test]
fn single_pin_net_is_floating() {
    let reg = registry();
    let mut eng = EngineBuilder::new("gnd");
    eng.net("a").net("b");
    let params = BTreeMap::new();
    let spec = |r: &'static str| ComponentSpec {
        refdes: r,
        part_id: "generic.resistor",
        params: &params,
        firmware: None,
        pins: &[],
        seed: 0,
    };
    let r1 = reg.build("rust:resistor", spec("R1")).unwrap();
    let r2 = reg.build("rust:resistor", spec("R2")).unwrap();
    eng.add(r1, &[("1", "a"), ("2", "gnd")], &[], 0).unwrap();
    eng.add(r2, &[("1", "a"), ("2", "b")], &[], 0).unwrap();
    let err = eng.elaborate().err().expect("b is floating").to_string();
    assert!(err.contains("'b'") && err.contains("R2.2"), "{err}");
}
