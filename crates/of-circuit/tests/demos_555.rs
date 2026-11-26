//! The 555 demo circuits (`circuits/demos/ne555-*.circuit.toml`) run
//! headless and read back against the NE555 data sheet (TI SLFS022K) and
//! the RC timing they are built for.
//!
//! An astable 555 has no DC operating point (the capacitor charges and
//! discharges for ever), so the files start cold (`[sim] uic = true`):
//! every net at 0 V and the supply applied at t = 0, as when a bench supply
//! is switched on, and the timer starts from an empty capacitor.
#![cfg(feature = "engine")]

use std::sync::Arc;

use of_circuit::build::build;
use of_circuit::embedded::EmbeddedResolver;
use of_circuit::library::PartsIndex;
use of_circuit::parse;
use of_components::analog::{Env, Library};
use of_components::{Corner, register_builtin};
use of_core::SimTime;
use of_engine::Registry;
use of_instruments::{Probeable, Signal};

fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

fn demo(name: &str) -> String {
    let p = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(format!("../../circuits/demos/{name}.circuit.toml"));
    std::fs::read_to_string(p).unwrap()
}

/// Run `name` from its cold start for `until` seconds,
/// sampling `signals` every `dt`; returns the time axis and one column per
/// signal.
/// `events` set host parameters `(time, ref, name, value)` on the way.
fn run(
    name: &str,
    until: f64,
    dt: f64,
    signals: &[&str],
    events: &[(f64, &str, &str, f64)],
) -> (Vec<f64>, Vec<Vec<f64>>) {
    run_src(&demo(name), until, dt, signals, events)
}

/// [`run`] on circuit-file source `src`.
fn run_src(
    src: &str,
    until: f64,
    dt: f64,
    signals: &[&str],
    events: &[(f64, &str, &str, f64)],
) -> (Vec<f64>, Vec<Vec<f64>>) {
    let f = parse(src).unwrap();
    let parts = PartsIndex::new(of_parts::load_dir(&parts_dir()).unwrap().into_iter().map(|(_, p)| p));
    let lib = Arc::new(Library::load_dir(&parts_dir()).unwrap());
    let mut reg = Registry::new();
    register_builtin(&mut reg, lib, Corner::Typical, Env::default());
    let mut b = build(src, &f, &parts, &reg, &EmbeddedResolver::new()).unwrap();
    let sigs: Vec<Signal> = signals.iter().map(|s| s.parse().unwrap()).collect();
    let (mut t, mut cols) = (Vec::new(), vec![Vec::new(); sigs.len()]);
    let mut k = 0u64;
    loop {
        let now = k as f64 * dt;
        if now > until {
            break;
        }
        b.session.run_until(SimTime::from_secs_f64(now)).unwrap();
        for &(at, r, p, v) in events {
            if at > now - dt / 2.0 && at <= now + dt / 2.0 {
                b.session.set_param(r, p, v).unwrap();
                b.session.commit().unwrap();
            }
        }
        t.push(now);
        for (c, s) in cols.iter_mut().zip(&sigs) {
            c.push(
                b.session
                    .value(&s.to_string())
                    .unwrap_or_else(|| b.session.sample(s).unwrap()),
            );
        }
        k += 1;
    }
    (t, cols)
}

/// Rising (`true`) and falling crossings of `level`, interpolated.
fn crossings(t: &[f64], v: &[f64], level: f64) -> Vec<(f64, bool)> {
    let mut out = Vec::new();
    for k in 1..t.len() {
        let (a, b) = (v[k - 1] - level, v[k] - level);
        if (a < 0.0) != (b < 0.0) {
            out.push((t[k - 1] + a / (a - b) * (t[k] - t[k - 1]), b > a));
        }
    }
    out
}

fn within(got: f64, want: f64, rel: f64) -> bool {
    (got - want).abs() <= rel * want.abs()
}

#[test]
fn astable_flasher_blinks_at_the_rc_rate() {
    // R1 1 k, R2 10 k, C1 10 uF at 5 V: high ln 2 (R1 + R2) C, low toward the
    // discharge saturation (SLFS022K 5.5: 0.15 V typ at 8 mA, 18.75 ohm).
    let (t, c) = run("ne555-astable", 0.8, 20e-6, &["v(out)", "v(cap)", "i(R3)"], &[]);
    let x: Vec<_> = crossings(&t, &c[0], 2.0).into_iter().filter(|x| x.0 > 0.2).collect();
    let rise: Vec<f64> = x.iter().filter(|x| x.1).map(|x| x.0).collect();
    let fall: Vec<f64> = x.iter().filter(|x| !x.1).map(|x| x.0).collect();
    assert!(rise.len() >= 3, "{x:?}");
    let period = (rise[rise.len() - 1] - rise[0]) / (rise.len() - 1) as f64;
    let high = fall.iter().find(|&&f| f > rise[0]).unwrap() - rise[0];
    let (r1, r2, cap, vcc, rd) = (1e3, 10e3, 10e-6, 5.0, 0.15 / 8e-3);
    let want_high = (r1 + r2) * cap * std::f64::consts::LN_2;
    let vd = vcc * rd / (r1 + rd);
    let want_low = (r2 + r1 * rd / (r1 + rd)) * cap * ((2.0 * vcc / 3.0 - vd) / (vcc / 3.0 - vd)).ln();
    println!(
        "astable: period {period} s (exact {}), high {high} s (exact {want_high})",
        want_high + want_low
    );
    assert!(within(high, want_high, 0.005), "high {high} vs {want_high}");
    assert!(within(period, want_high + want_low, 0.005), "period {period}");
    // SLFS022 (1992) p. 5: 2.25 % typical astable timing error.
    let datasheet = 1.0 / (1.44 / ((r1 + 2.0 * r2) * cap));
    assert!(within(period, datasheet, 0.0225), "period {period} vs {datasheet}");
    // The capacitor swings between VCC / 3 and 2 VCC / 3 (5.5: 1.67 V and
    // 3.3 V typ at 5 V).
    let late: Vec<f64> = t
        .iter()
        .zip(&c[1])
        .filter(|(t, _)| **t > 0.3)
        .map(|(_, v)| *v)
        .collect();
    let (lo, hi) = late
        .iter()
        .fold((f64::MAX, f64::MIN), |(a, b), &v| (a.min(v), b.max(v)));
    println!("astable: cap {lo} .. {hi} V");
    assert!(
        (lo - vcc / 3.0).abs() < 0.02 && (hi - 2.0 * vcc / 3.0).abs() < 0.02,
        "{lo} .. {hi}"
    );
    // LED current while lit: VOH = 5 - 0.9 - 8 ohm x I (5.5 table fit:
    // 3.3 V at -100 mA, VCC = 5 V), through 220 ohm and the red LED (about
    // 1.85 V at 10 mA): about 9 mA.
    let lit = t
        .iter()
        .zip(&c[2])
        .filter(|(t, _)| **t > rise[0] + 0.01 && **t < rise[0] + 0.05)
        .map(|(_, i)| *i);
    let i_on = lit.clone().sum::<f64>() / lit.count() as f64;
    println!("astable: LED current lit {i_on} A");
    assert!((0.008..0.010).contains(&i_on), "{i_on}");
}

#[test]
fn monostable_button_lights_the_led_for_1_1_rc() {
    // SLFS022K 6.3.1: tw = 1.1 RA C (ln 3 = 1.0986 exactly for the 1/3 and
    // 2/3 thresholds of 5.5); SLFS022 (1992) p. 5: 1 % typical monostable
    // timing error. SW1 is pressed at 20 ms (with contact bounce) and
    // released at 50 ms, before the pulse ends.
    let events = [(0.020, "SW1", "closed", 1.0), (0.050, "SW1", "closed", 0.0)];
    let (t, c) = run(
        "ne555-monostable",
        0.25,
        20e-6,
        &["v(out)", "v(cap)", "v(trig)"],
        &events,
    );
    let x = crossings(&t, &c[0], 2.0);
    assert_eq!(x.len(), 2, "one pulse: {x:?}");
    let width = x[1].0 - x[0].0;
    let (ra, cap) = (10e3, 10e-6);
    println!(
        "monostable: pulse {width} s from {} s (exact {} s)",
        x[0].0,
        ra * cap * 3f64.ln()
    );
    assert!((x[0].0 - 0.020).abs() < 1e-4, "starts at the press: {}", x[0].0);
    assert!(within(width, ra * cap * 3f64.ln(), 0.005), "width {width}");
    assert!(within(width, 1.1 * ra * cap, 0.01), "width {width} vs 1.1 RC");
    // The capacitor peaks at 2 VCC / 3 (3.33 V typ at 5 V) and is emptied
    // by the discharge transistor afterwards.
    let peak = c[1].iter().copied().fold(f64::MIN, f64::max);
    println!("monostable: cap peak {peak} V, final {} V", c[1][c[1].len() - 1]);
    assert!((peak - 10.0 / 3.0).abs() < 0.02, "{peak}");
    assert!(c[1][c[1].len() - 1] < 0.01);
}

/// Time for C1 to swing between VCC/3 and 2VCC/3 through a resistance `r`
/// and a 1N4148 (the part's IS, N and RS at 27 C) against `v_far`, the
/// voltage at the far end of `r` (above the swing when charging, below it
/// when discharging): t = integral of C dv / i(v), with i solving
/// |v_far - v| = i r + N VT ln(1 + i / IS) (Simpson, 2000 panels).
fn diode_rc_time(c: f64, vcc: f64, r: f64, v_far: f64) -> f64 {
    const IS: f64 = 4e-9;
    const N: f64 = 1.9;
    const RS: f64 = 0.5;
    const VT: f64 = 0.025_864_917;
    let current = |drop: f64| {
        let (mut lo, mut hi) = (0.0, drop / (r + RS));
        for _ in 0..100 {
            let m = 0.5 * (lo + hi);
            if m * (r + RS) + N * VT * (m / IS).ln_1p() > drop {
                hi = m;
            } else {
                lo = m;
            }
        }
        0.5 * (lo + hi)
    };
    let f = |v: f64| c / current((v_far - v).abs());
    let (a, b, n) = (vcc / 3.0, 2.0 * vcc / 3.0, 2000);
    let h = (b - a) / n as f64;
    let mut s = f(a) + f(b);
    for k in 1..n {
        s += f(a + k as f64 * h) * if k % 2 == 1 { 4.0 } else { 2.0 };
    }
    s * h / 3.0
}

/// The two-diode duty control: C1 charges through R1 + RA and D1, and
/// discharges through D2 and RB into DISCH, which also sinks R1's current
/// (5 mA): the discharge saturation (RDIS) holds DISCH at about 93 mV, the
/// Thevenin of R1 and RDIS. The exact times integrate the diode law along
/// the swing (`diode_rc_time`); the textbook (R1 + RA) / (R1 + RA + RB)
/// ignores the diode drops and that 93 mV (which lengthens the low time
/// about 5 %), so it is only a 2-point check.
#[test]
fn pwm_dimmer_duty_follows_the_pot() {
    const R1: f64 = 1e3;
    const RPOT: f64 = 10e3;
    const RDIS: f64 = 18.75; // 0.15 V at 8 mA, SLFS022K 5.5
    const WINDOW: f64 = 10e-3;
    let positions = [0.1, 0.3, 0.5, 0.7, 0.9];
    let events: Vec<(f64, &str, &str, f64)> = positions
        .iter()
        .enumerate()
        .map(|(k, &x)| (k as f64 * WINDOW, "RV1", "position", x))
        .collect();
    let until = positions.len() as f64 * WINDOW;
    let (t, cols) = run("ne555-pwm-dimmer", until, 1e-6, &["v(out)", "i(R3)"], &events);
    let out = &cols[0];
    for (k, &x) in positions.iter().enumerate() {
        // The last 5 ms of each 10 ms window (well past the step).
        let (a, b) = (k as f64 * WINDOW + 5e-3, (k + 1) as f64 * WINDOW);
        let i0 = t.iter().position(|&s| s >= a).unwrap();
        let i1 = t.iter().position(|&s| s >= b).unwrap_or(t.len() - 1);
        let edges = crossings(&t[i0..=i1], &out[i0..=i1], 2.5);
        let rises: Vec<f64> = edges.iter().filter(|e| e.1).map(|e| e.0).collect();
        let falls: Vec<f64> = edges.iter().filter(|e| !e.1).map(|e| e.0).collect();
        assert!(rises.len() >= 3, "pot {x}: {} rising edges", rises.len());
        let n = rises.len() - 1;
        let period = (rises[n] - rises[0]) / n as f64;
        let high: f64 = rises[..n]
            .iter()
            .map(|r| falls.iter().find(|&&f| f > *r).unwrap() - r)
            .sum::<f64>()
            / n as f64;
        let duty = high / period;
        let (ra, rb) = (x * RPOT, (1.0 - x) * RPOT);
        let theory = (R1 + ra) / (R1 + ra + rb + RDIS);
        let (vcc, c) = (5.0, 1e-7);
        let t_high = diode_rc_time(c, vcc, R1 + ra, vcc);
        let r_dis = R1 * RDIS / (R1 + RDIS);
        let t_low = diode_rc_time(c, vcc, rb + r_dis, vcc * RDIS / (R1 + RDIS));
        let (exact_p, exact_d) = (t_high + t_low, t_high / (t_high + t_low));
        // Mean over whole periods.
        let (j0, j1) = (
            t.iter().position(|&s| s >= rises[0]).unwrap(),
            t.iter().position(|&s| s >= rises[n]).unwrap(),
        );
        let mean_i = cols[1][j0..j1].iter().sum::<f64>() / (j1 - j0) as f64;
        println!(
            "pot {x:.1}: period {:.4} ms (exact {:.4}), duty {:.4} (exact {exact_d:.4}, {:+.3} points; textbook {theory:.4}, {:+.2}), mean LED {:.3} mA",
            period * 1e3,
            exact_p * 1e3,
            duty,
            (duty - exact_d) * 100.0,
            (duty - theory) * 100.0,
            mean_i * 1e3
        );
        // Exact: what is left is the 1 us sampling of the edges and the
        // timer's own delays (about 0.1 point, 0.4 % of the period).
        assert!(
            (duty - exact_d).abs() < 0.002,
            "pot {x}: duty {duty} vs exact {exact_d}"
        );
        assert!(
            (period / exact_p - 1.0).abs() < 0.005,
            "pot {x}: period {period} vs exact {exact_p}"
        );
        assert!(
            (duty - theory).abs() < 0.02,
            "pot {x}: duty {duty} vs textbook {theory}"
        );
        // The period is (R1 + RPOT + RDIS) C ln(...) whatever the pot: about
        // 1 kHz (0.94 ms to 1.06 ms).
        assert!((0.94e-3..1.06e-3).contains(&period), "pot {x}: period {period}");
        // The LED's average current follows the duty (about 9.5 mA lit).
        assert!(
            (mean_i / duty - 9.5e-3).abs() < 0.5e-3,
            "pot {x}: {mean_i} A at duty {duty}"
        );
    }
}

/// A cold start (`[sim] uic`) has no solved rail at t = 0 (every net is at
/// 0 V), so a commit there must not read the supply as shorted: the PSU
/// would drop into constant current, high impedance at its current limit,
/// and drive the load far above its set point (100 V here) or stall the
/// solver on forward-biased diodes. It stays in CV and C1 charges through
/// R1 and D1 to 5 V less the drop.
#[test]
fn cold_start_commit_keeps_the_supply_in_cv() {
    let src = r#"format = 1

[circuit]
name = "cold start"
ground = "gnd"

[sim]
max_step = 1e-5
uic = true

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 1000 }

[[component]]
ref = "D1"
part = "onsemi.1n4148"

[[component]]
ref = "C1"
part = "generic.capacitor-mlcc"
params = { capacitance = 1e-7 }

[[component]]
ref = "PSU"
part = "bench.psu"
params = { current_limit = 0.1, on = true, series_r = 0.1, volts = 5 }

[[net]]
name = "vcc"
pins = ["PSU.+", "R1.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["PSU.-", "C1.2"]

[[net]]
name = "a"
pins = ["R1.2", "D1.A"]

[[net]]
name = "cap"
pins = ["D1.K", "C1.1"]
"#;
    // The event at t = 0 makes the commit there.
    let (t, cols) = run_src(
        src,
        1e-3,
        1e-5,
        &["v(vcc)", "v(cap)"],
        &[(0.0, "R1", "resistance", 1000.0)],
    );
    let vmax = cols[0].iter().copied().fold(f64::MIN, f64::max);
    assert!(vmax < 5.0 + 1e-6, "rail peaked at {vmax} V");
    let cap = *cols[1].last().unwrap();
    // 10 RC (1 ms) in: 5 V less the 1N4148 drop at a few uA.
    assert!((4.4..4.7).contains(&cap), "v(cap) = {cap} at {} s", t.last().unwrap());
}

/// In constant current a bench supply never drives its rail above the set
/// point. A 10 uF load on a 10 mA limit charges at the limit (1 V/ms) and
/// stops at 5 V; the old supply (a bare current source in CC, regulated
/// only at commits) reached 24.6 V here in 20 ms with a single commit.
#[test]
fn cc_mode_never_exceeds_the_set_point() {
    let src = r#"format = 1

[circuit]
name = "cc compliance"
ground = "gnd"

[sim]
max_step = 1e-5
uic = true

[[component]]
ref = "C1"
part = "generic.capacitor-mlcc"
params = { capacitance = 1e-5 }

[[component]]
ref = "R1"
part = "generic.resistor"
params = { resistance = 100000 }

[[component]]
ref = "PSU"
part = "bench.psu"
params = { current_limit = 0.01, on = true, series_r = 0.1, volts = 5 }

[[net]]
name = "vcc"
pins = ["PSU.+", "C1.1", "R1.1"]

[[net]]
name = "gnd"
kind = "ground"
pins = ["PSU.-", "C1.2", "R1.2"]
"#;
    let (_, cols) = run_src(
        src,
        20e-3,
        1e-4,
        &["v(vcc)", "i(R1)"],
        &[(1e-4, "R1", "resistance", 100000.0)],
    );
    let v = &cols[0];
    let vmax = v.iter().copied().fold(f64::MIN, f64::max);
    println!(
        "CC: v(1 ms) {:.6} V, rail max {vmax:.9} V, end {:.9} V",
        v[10],
        v.last().unwrap()
    );
    assert!(vmax <= 5.0, "rail rose to {vmax} V in CC");
    // Charging at the limit: about 1 V at 1 ms (less the 100 kohm's share).
    assert!((v[10] - 1.0).abs() < 0.01, "v(1 ms) = {}", v[10]);
    // Settled in CV: 5 V less 50 uA x 0.1 ohm.
    assert!(
        (v.last().unwrap() - (5.0 - 5e-6)).abs() < 1e-6,
        "end {}",
        v.last().unwrap()
    );
}

/// The app builds the 555 demos from the embedded parts and SPICE
/// libraries alone (`of_circuit::demos`, no repository `parts/`).
#[cfg(feature = "demos")]
#[test]
fn the_555_demos_build_from_the_embedded_parts() {
    use of_circuit::demos::{demo, part_files, parts, resolver, spice_model};
    let mut lib = Library::new();
    for p in part_files().unwrap() {
        let spice = spice_model(&p.part.model).map(str::to_owned);
        lib.add(p, spice).unwrap();
    }
    let lib = Arc::new(lib);
    let index = parts().unwrap();
    for name in ["ne555-astable", "ne555-monostable", "ne555-pwm-dimmer"] {
        let d = demo(name).unwrap();
        let f = parse(d.text).unwrap();
        let mut reg = Registry::new();
        register_builtin(&mut reg, Arc::clone(&lib), Corner::Typical, Env::default());
        let mut b = build(d.text, &f, &index, &reg, &resolver()).unwrap_or_else(|e| panic!("{name}: {e}"));
        b.session.run_until(SimTime::from_secs_f64(5e-3)).unwrap();
        // The output is high after a cold start (astables) or idle low
        // (monostable): either way a solved, finite node.
        let out = b.session.value("v(out)").unwrap();
        assert!(out.is_finite() && (-0.1..5.1).contains(&out), "{name}: v(out) = {out}");
    }
}
