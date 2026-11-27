//! NE555 macromodel, placed through the parts bridge, against TI's
//! SLFS022K data sheet (25 °C, VCC 5 to 15 V, typical values).
//!
//! Timing tolerances: 5.5 gives the THRES and TRIG levels as exactly 2/3
//! and 1/3 of VCC (10 V and 5 V typ at 15 V), so a typical device times the
//! RC intervals between those levels. Two things move a real part off the
//! textbook constants of 6.3.1 and 6.3.2: the constants are rounded
//! (1.1 for ln 3 = 1.0986, 1.44 for 1 / ln 2 = 1.4427), and in an astable
//! the discharge transistor does not reach 0 V (5.5: 0.15 V typ at 8 mA),
//! so the capacitor discharges toward that voltage and the low time grows
//! (+4 % for RA = 1 kOhm at 5 V). The tests therefore check:
//! - the exact interval with the data sheet's discharge voltage
//!   (0.15 V / 8 mA = 18.75 Ohm) to 0.5 %, covering what is left: the pin
//!   currents (0.5 uA TRIG against >= 150 uA of charge current, 0.3 %), the
//!   1 mV comparator band and the time step;
//! - the textbook formulas to 1 % (monostable) and 2.25 % (astable): the
//!   NE555 typical "initial error of timing interval" in TI SLFS022
//!   (September 1973, revised February 1992), page 5, table "operating
//!   characteristics, VCC = 5 V and 15 V" (1 % typ, 3 % max monostable;
//!   2.25 % typ astable; 25 °C). SLFS022K no longer has that row. TI
//!   defines the figure as the spread about a process run's average, with
//!   RA = 1 or 2 kOhm to 100 kOhm and C = 0.1 uF, so it bounds how far a
//!   typical part lands from the nominal interval rather than the textbook
//!   constants themselves; the tests use smaller C to keep runs short.

mod common;

use of_analog::netlist::{BuildEnv, add_deck};
use of_analog::{Circuit, Options, analysis};
use of_components::analog::{Board, Placement, build};

/// The timer with pins on the named nodes (`0` is ground) and `stimulus`.
fn timer(pins: &[(&str, &str)], stimulus: &str) -> Circuit {
    let lib = common::library();
    let board = Board {
        placements: vec![Placement::new("U1", "ti.ne555", pins)],
        ..Board::default()
    };
    let mut b = build(&lib, &board).unwrap();
    assert!(b.warnings.is_empty(), "{:?}", b.warnings);
    let deck = of_netlist::spice::parse_str(&format!("stimulus\n{stimulus}\n.end\n")).unwrap();
    add_deck(&mut b.circuit, &deck, &BuildEnv::default(), &mut Vec::new()).unwrap();
    b.circuit
}

/// Astable (6.3.2, Figure 6-5): TRIG and THRES on the timing capacitor.
const ASTABLE: &[(&str, &str)] = &[
    ("GND", "0"),
    ("TRIG", "thr"),
    ("OUT", "out"),
    ("RESET", "rst"),
    ("CONT", "cont"),
    ("THRES", "thr"),
    ("DISCH", "dis"),
    ("VCC", "vcc"),
];

/// Monostable (6.3.1, Figure 6-2): THRES and DISCH on the capacitor.
const MONO: &[(&str, &str)] = &[
    ("GND", "0"),
    ("TRIG", "trig"),
    ("OUT", "out"),
    ("RESET", "rst"),
    ("CONT", "cont"),
    ("THRES", "thr"),
    ("DISCH", "thr"),
    ("VCC", "vcc"),
];

fn run(c: &mut Circuit, tstop: f64) -> (Vec<f64>, Vec<f64>) {
    let req = analysis::TranRequest {
        tstep: 50e-9,
        tstop,
        tmax: 50e-9,
        ..Default::default()
    };
    let w = analysis::transient(c, &Options::default(), &req).unwrap();
    (w.x.clone(), w.signal("v(out)").unwrap().to_vec())
}

/// Times `v` crosses `level`, with the direction (`true` rising).
fn crossings(t: &[f64], v: &[f64], level: f64) -> Vec<(f64, bool)> {
    let mut out = Vec::new();
    for k in 1..t.len() {
        let (a, b) = (v[k - 1] - level, v[k] - level);
        if (a < 0.0) != (b < 0.0) {
            let f = a / (a - b);
            out.push((t[k - 1] + f * (t[k] - t[k - 1]), b > a));
        }
    }
    out
}

/// High time and period of the last full cycles after `settle`.
fn cycle(t: &[f64], v: &[f64], vcc: f64, settle: f64) -> (f64, f64) {
    let x: Vec<_> = crossings(t, v, vcc / 2.0)
        .into_iter()
        .filter(|c| c.0 > settle)
        .collect();
    let rise: Vec<f64> = x.iter().filter(|c| c.1).map(|c| c.0).collect();
    let fall: Vec<f64> = x.iter().filter(|c| !c.1).map(|c| c.0).collect();
    assert!(rise.len() >= 3, "{rise:?}");
    let period = (rise[rise.len() - 1] - rise[0]) / (rise.len() - 1) as f64;
    let f = fall.iter().find(|&&f| f > rise[0]).unwrap();
    (f - rise[0], period)
}

fn within(got: f64, want: f64, rel: f64) -> bool {
    (got - want).abs() <= rel * want.abs()
}

const RA: f64 = 1e3;
const RB: f64 = 10e3;
const C: f64 = 10e-9;

/// Discharge on-resistance from 5.5: 0.15 V typ at 8 mA.
const RDIS: f64 = 0.15 / 8e-3;

/// Exact astable high and low times between `vl` and `vu` at `vcc`, the
/// low one toward the discharge transistor's saturation voltage.
fn astable_exact(vcc: f64, vl: f64, vu: f64) -> (f64, f64) {
    let vdis = vcc * RDIS / (RA + RDIS);
    let high = (RA + RB) * C * ((vcc - vl) / (vcc - vu)).ln();
    let low = (RB + RA * RDIS / (RA + RDIS)) * C * ((vu - vdis) / (vl - vdis)).ln();
    (high, low)
}

fn astable_deck(extra: &str) -> String {
    format!("VCC vcc 0 PWL(0 0 1u 5)\nVR rst vcc 0\nRA vcc dis {RA}\nRB dis thr {RB}\nC1 thr 0 {C}\n{extra}")
}

#[test]
fn astable_frequency_and_duty_cycle() {
    // 6.3.2: f = 1.44 / ((RA + 2 RB) C), duty (RA + RB) / (RA + 2 RB).
    let mut c = timer(ASTABLE, &astable_deck("CC cont 0 10n"));
    let (t, v) = run(&mut c, 1.2e-3);
    let (high, period) = cycle(&t, &v, 5.0, 0.5e-3);
    let (want_high, want_low) = astable_exact(5.0, 5.0 / 3.0, 10.0 / 3.0);
    assert!(within(high, want_high, 0.005), "high {high} vs {want_high}");
    assert!(within(period, want_high + want_low, 0.005), "period {period}");
    let f = 1.0 / period;
    let datasheet = 1.44 / ((RA + 2.0 * RB) * C);
    assert!(within(f, datasheet, 0.0225), "f {f} Hz vs {datasheet}");
    let duty = high / period;
    let textbook = (RA + RB) / (RA + 2.0 * RB);
    assert!((duty - want_high / (want_high + want_low)).abs() < 0.005, "duty {duty}");
    // The high time is the ideal one, so the textbook duty misses only by
    // the low-time share of the 2.25 % astable error: under 1.5 points.
    assert!((duty - textbook).abs() < 0.015, "duty {duty} vs {textbook}");
}

#[test]
fn astable_timing_does_not_depend_on_max_step() {
    // The astable with C1 = 10 uF (period about 148 ms) on a 100 us maximum
    // step: the comparator and latch switches must flip at their threshold
    // crossings, not at the end of whichever step crossed them.
    let deck = astable_deck("CC cont 0 10n").replace(&format!("C1 thr 0 {C}"), "C1 thr 0 10u");
    let mut c = timer(ASTABLE, &deck);
    let req = analysis::TranRequest {
        tstep: 1e-4,
        tstop: 0.8,
        tmax: 1e-4,
        ..Default::default()
    };
    let w = analysis::transient(&mut c, &Options::default(), &req).unwrap();
    let (t, v) = (w.x.clone(), w.signal("v(out)").unwrap().to_vec());
    let (high, period) = cycle(&t, &v, 5.0, 0.2);
    let (h0, l0) = astable_exact(5.0, 5.0 / 3.0, 10.0 / 3.0);
    let k = 10e-6 / C;
    assert!(within(high, h0 * k, 0.005), "high {high} vs {}", h0 * k);
    assert!(
        within(period, (h0 + l0) * k, 0.005),
        "period {period} vs {}",
        (h0 + l0) * k
    );
}

#[test]
fn monostable_width_is_1_1_rc() {
    // 6.3.1: tw = 1.1 RA C after TRIG falls below VCC / 3.
    let r = 10e3;
    let deck = format!(
        "VCC vcc 0 PWL(0 0 1u 5)\nVR rst vcc 0\nRA vcc thr {r}\nC1 thr 0 {C}\nCC cont 0 10n\nVT trig 0 PULSE(5 0 100u 10n 10n 5u 1)"
    );
    let mut c = timer(MONO, &deck);
    let (t, v) = run(&mut c, 400e-6);
    let x: Vec<_> = crossings(&t, &v, 2.5).into_iter().filter(|c| c.0 > 50e-6).collect();
    assert_eq!(x.len(), 2, "one pulse: {x:?}");
    let width = x[1].0 - x[0].0;
    assert!(x[0].1 && !x[1].1);
    assert!(within(width, 1.1 * r * C, 0.01), "width {width}");
    assert!((x[0].0 - 100e-6).abs() < 1e-6, "starts at the trigger: {}", x[0].0);
}

#[test]
fn reset_overrides_and_its_threshold() {
    // Table 6-1: RESET low forces the output low over TRIG; 5.5 puts the
    // RESET level at 0.7 V typ (0.3 to 1 V).
    let op = |reset: f64| {
        let mut c = timer(
            ASTABLE,
            &format!("VCC vcc 0 5\nVR rst 0 {reset}\nVT thr 0 0\nRA vcc dis 1k"),
        );
        analysis::operating_point(&mut c, &Options::default())
            .unwrap()
            .get("v(out)")
            .unwrap()
    };
    assert!(op(0.5) < 0.1, "RESET 0.5 V holds the output low");
    assert!(op(1.0) > 3.5, "RESET 1 V lets TRIG set it");
    // In an astable: RESET low stops it and empties the capacitor; released,
    // it runs again.
    let deck = astable_deck("CC cont 0 10n").replace("VR rst vcc 0", "VR rst 0 PWL(0 5 400u 5 401u 0 600u 0 601u 5)");
    let mut c = timer(ASTABLE, &deck);
    let req = analysis::TranRequest {
        tstep: 50e-9,
        tstop: 1.0e-3,
        tmax: 50e-9,
        ..Default::default()
    };
    let w = analysis::transient(&mut c, &Options::default(), &req).unwrap();
    let (t, out, thr) = (&w.x, w.signal("v(out)").unwrap(), w.signal("v(thr)").unwrap());
    for k in 0..t.len() {
        if (420e-6..600e-6).contains(&t[k]) {
            assert!(out[k] < 0.1, "out {} at {}", out[k], t[k]);
        }
    }
    // The capacitor empties through RB toward the discharge voltage while
    // RESET is low (5.5: DISCH on when RESET is low).
    let at = |x: f64| thr[t.iter().position(|&s| s >= x).unwrap()];
    let vdis = 5.0 * RDIS / (RA + RDIS);
    let tau = (RB + RA * RDIS / (RA + RDIS)) * C;
    let want = vdis + (at(420e-6) - vdis) * (-(599e-6 - 420e-6) / tau).exp();
    assert!((at(599e-6) - want).abs() < 0.01, "THRES {} vs {want}", at(599e-6));
    let after = crossings(t, out, 2.5)
        .into_iter()
        .filter(|c| c.0 > 601e-6 && c.1)
        .count();
    assert!(after >= 2, "runs again");
}

/// The undervoltage lockout (a model assumption, see ne555.lib: SLFS022K
/// gives no figure). With TRIG held low, so the latch would set, the output
/// stays low and DISCH on below 2.3 V and the latch runs above 2.5 V; on a
/// slow VCC ramp the release comes near 2.5 V rising and the lockout near
/// 2.3 V falling (the 10 us sense filter lags 0.05 V at 5 V/ms).
#[test]
fn undervoltage_lockout_holds_the_latch_reset() {
    let op = |vcc: f64| {
        let mut c = timer(
            ASTABLE,
            &format!("VCC vcc 0 {vcc}\nVR rst vcc 0\nVT thr 0 0\nRA vcc dis 1k"),
        );
        let r = analysis::operating_point(&mut c, &Options::default()).unwrap();
        (r.get("v(out)").unwrap(), r.get("v(dis)").unwrap())
    };
    for vcc in [0.5, 1.0, 2.0, 2.25] {
        let (out, dis) = op(vcc);
        assert!(out < 0.05, "{vcc} V: out {out}");
        assert!(dis < 0.05 * vcc, "{vcc} V: DISCH on, {dis} V");
    }
    for vcc in [2.6, 3.0, 4.5] {
        let (out, dis) = op(vcc);
        assert!((out - (vcc - 0.9)).abs() < 0.05, "{vcc} V: out {out}");
        assert!(dis > 0.99 * vcc, "{vcc} V: DISCH off, {dis} V");
    }
    // 0 to 5 V in 1 ms, held, then back to 0 in 1 ms.
    let mut c = timer(
        ASTABLE,
        "VCC vcc 0 PWL(0 0 1m 5 2m 5 3m 0)\nVR rst vcc 0\nVT thr 0 0\nRA vcc dis 1k",
    );
    let req = analysis::TranRequest {
        tstep: 1e-6,
        tstop: 3e-3,
        tmax: 1e-6,
        ..Default::default()
    };
    let w = analysis::transient(&mut c, &Options::default(), &req).unwrap();
    let (t, out, vcc) = (&w.x, w.signal("v(out)").unwrap(), w.signal("v(vcc)").unwrap());
    let edges = crossings(t, out, 0.7);
    assert_eq!(edges.len(), 2, "{edges:?}");
    let at = |x: f64| vcc[t.iter().position(|&s| s >= x).unwrap()];
    let (up, down) = (at(edges[0].0), at(edges[1].0));
    println!("UVLO: released at VCC {up:.3} V rising, locked at {down:.3} V falling");
    assert!(edges[0].1 && (2.5..2.6).contains(&up), "release at {up} V");
    assert!(!edges[1].1 && (2.2..2.3).contains(&down), "lockout at {down} V");
}

#[test]
fn control_voltage_moves_both_thresholds() {
    // CONT driven to 4 V at VCC = 5 V: THRES at 4 V, TRIG at 2 V (pin
    // table: TRIG < 1/2 CONT). High RA+RB from 2 to 4 V toward 5 V: ln 3;
    // low RB from 4 to 2 V: ln 2.
    let mut c = timer(ASTABLE, &astable_deck("VC cont 0 4"));
    let (t, v) = run(&mut c, 1.5e-3);
    let (high, period) = cycle(&t, &v, 5.0, 0.5e-3);
    let (want_high, want_low) = astable_exact(5.0, 2.0, 4.0);
    assert!(within(high, want_high, 0.005), "high {high} vs {want_high}");
    assert!(within(period, want_high + want_low, 0.005), "period {period}");
    // Against the ideal ln 3 / ln 2 intervals, to the astable 2.25 %.
    let ideal = (RA + RB) * C * 3f64.ln() + RB * C * std::f64::consts::LN_2;
    assert!(within(period, ideal, 0.0225), "period {period} vs {ideal}");
    // Open CONT sits at 2/3 VCC (5.5: 3.3 V typ at 5 V, 10 V at 15 V).
    for vcc in [5.0, 15.0] {
        let mut c = timer(
            ASTABLE,
            &format!("VCC vcc 0 {vcc}\nVR rst vcc 0\nVT thr 0 {vcc}\nRA vcc dis 1k"),
        );
        let v = analysis::operating_point(&mut c, &Options::default())
            .unwrap()
            .get("v(cont)")
            .unwrap();
        assert!(within(v, vcc * 2.0 / 3.0, 0.005), "{vcc} V: CONT {v}");
    }
}

#[test]
fn output_drops_discharge_and_supply_current() {
    // 5.5 at VCC = 15 V: VOH 13.3 V typ (12.75 min) at -100 mA, 12.5 V at
    // -200 mA; VOL 0.4 V typ (0.75 max) at 50 mA, 0.1 V at 10 mA.
    let op = |deck: &str, sig: &str| {
        let mut c = timer(ASTABLE, deck);
        analysis::operating_point(&mut c, &Options::default())
            .unwrap()
            .get(sig)
            .unwrap()
    };
    let high = |vcc: f64, load: &str| {
        op(
            &format!("VCC vcc 0 {vcc}\nVR rst vcc 0\nVT thr 0 0\nRA vcc dis 1k\n{load}"),
            "v(out)",
        )
    };
    let low = |vcc: f64, load: &str| {
        op(
            &format!("VCC vcc 0 {vcc}\nVR rst vcc 0\nVT thr 0 {vcc}\nRA vcc dis 1k\n{load}"),
            "v(out)",
        )
    };
    let voh = high(15.0, "IL out 0 100m");
    assert!((12.75..13.5).contains(&voh) && within(voh, 13.3, 0.01), "VOH {voh}");
    assert!(within(high(15.0, "IL out 0 200m"), 12.5, 0.01));
    assert!(within(high(5.0, "IL out 0 100m"), 3.3, 0.02));
    let vol = low(15.0, "IL vcc out 50m");
    assert!(vol < 0.75 && within(vol, 0.4, 0.05), "VOL {vol}");
    assert!((0.05..0.25).contains(&low(15.0, "IL vcc out 10m")));
    // DISCH on: 0.15 V typ (0.4 max) at 8 mA, VCC = 5 V.
    let vd = op("VCC vcc 0 5\nVR rst vcc 0\nVT thr 0 5\nID vcc dis 8m", "v(dis)");
    assert!(vd < 0.4 && within(vd, 0.15, 0.05), "DISCH {vd}");
    // Supply current, no load (typ; max 13/15 mA at 15 V, 5/6 mA at 5 V),
    // with RESET at VCC; the reset pin's own current is included.
    for (vcc, trig, typ, max) in [
        (15.0, 0.0, 9e-3, 13e-3),
        (15.0, 15.0, 10e-3, 15e-3),
        (5.0, 0.0, 2e-3, 5e-3),
        (5.0, 5.0, 3e-3, 6e-3),
    ] {
        let i = -op(
            &format!("VCC vcc 0 {vcc}\nVR rst vcc 0\nVT thr 0 {trig}\nRA vcc dis 1meg"),
            "i(vcc)",
        );
        assert!(i < max && within(i, typ, 0.1), "{vcc} V, TRIG {trig}: {i} A");
    }
}
