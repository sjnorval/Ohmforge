//! Analytic-solution tests: each circuit has a closed-form answer.

use of_analog::analysis::{self, OpMethod};
use of_analog::netlist::{self, Built};
use of_analog::{AnalysisError, OpPoint, Waveforms};
use of_netlist::spice::{Analysis, parse_str};

const VT: f64 = 0.025_864_917; // kT/q at 27 °C, same constants as SPICE3

fn build(src: &str) -> Built {
    netlist::build(&parse_str(src).expect("parse")).expect("build")
}

fn op(src: &str) -> OpPoint {
    let mut b = build(src);
    analysis::operating_point(&mut b.circuit, &b.options).expect("op")
}

fn tran(src: &str) -> Waveforms {
    let mut b = build(src);
    let a = b
        .analyses
        .iter()
        .find(|a| matches!(a, Analysis::Tran { .. }))
        .cloned()
        .expect(".tran");
    let req = b.tran_request(&a).unwrap();
    analysis::transient(&mut b.circuit, &b.options, &req).expect("tran")
}

/// Largest absolute error between a waveform and an analytic function.
fn max_err(w: &Waveforms, name: &str, f: impl Fn(f64) -> f64) -> f64 {
    let y = w
        .signal(name)
        .unwrap_or_else(|| panic!("no signal {name}: {:?}", w.names));
    w.x.iter().zip(y).map(|(&t, &v)| (v - f(t)).abs()).fold(0.0, f64::max)
}

#[test]
fn resistive_divider_and_current_source() {
    let p = op("t\nV1 in 0 12\nR1 in a 2k\nR2 a 0 1k\nI1 0 b 1m\nR3 b 0 4.7k\n.op\n");
    assert!((p.get("v(a)").unwrap() - 4.0).abs() < 1e-9);
    assert!((p.get("v(b)").unwrap() - 4.7).abs() < 1e-9);
    // SPICE sign convention: current flows into the + terminal of V1.
    assert!((p.get("i(v1)").unwrap() + 4e-3).abs() < 1e-12);
    assert_eq!(p.method, OpMethod::Newton);
}

#[test]
fn controlled_sources() {
    let p = op("t\nV1 a 0 1\nR1 a 0 1k\n\
        E1 e 0 a 0 3\nRe e 0 1k\n\
        G1 0 g a 0 2m\nRg g 0 1k\n\
        F1 0 f V1 10\nRf f 0 100\n\
        H1 h 0 V1 500\nRh h 0 1k\n.op\n");
    assert!((p.get("v(e)").unwrap() - 3.0).abs() < 1e-9);
    assert!((p.get("v(g)").unwrap() - 2.0).abs() < 1e-9);
    // i(V1) = -1 mA, F1 pushes 10 * i(V1) from f to 0 ... into f: v(f) = -10 * -1m * 100 * -1.
    assert!((p.get("v(f)").unwrap() + 1.0).abs() < 1e-9, "{:?}", p.get("v(f)"));
    assert!((p.get("v(h)").unwrap() + 0.5).abs() < 1e-9);
}

#[test]
fn rc_step_trapezoidal_and_gear() {
    for method in ["", ".options method=gear\n"] {
        let w = tran(&format!(
            "rc\nV1 in 0 PULSE(0 1 0 1n 1n 1 2)\nR1 in out 1k\nC1 out 0 1u\n{method}.tran 10u 5m\n"
        ));
        let e = max_err(&w, "v(out)", |t| {
            if t <= 1e-9 {
                0.0
            } else {
                1.0 - (-(t - 0.5e-9) / 1e-3).exp()
            }
        });
        assert!(e < 2e-3, "{method}: max error {e}");
        assert!((w.at("v(out)", 5e-3).unwrap() - (1.0 - (-5.0f64).exp())).abs() < 1e-3);
    }
}

#[test]
fn rc_discharge_from_initial_condition() {
    let w = tran("rc\nR1 out 0 1k\nC1 out 0 1u ic=2\n.tran 10u 3m uic\n");
    let e = max_err(&w, "v(out)", |t| 2.0 * (-t / 1e-3).exp());
    assert!(e < 2e-3 * 2.0, "max error {e}");
}

#[test]
fn rl_step_current() {
    let w = tran("rl\nV1 in 0 PULSE(0 1 0 1p)\nR1 in a 1k\nL1 a 0 1m\n.tran 10n 5u\n");
    let e = max_err(&w, "i(l1)", |t| 1e-3 * (1.0 - (-t / 1e-6).exp()));
    assert!(e < 1e-3 * 1e-2, "max error {e}");
}

#[test]
fn series_rlc_underdamped_step() {
    // R = 10, L = 1 mH, C = 1 µF: alpha = 5000 /s, w0 = 31623 rad/s.
    let w = tran("rlc\nV1 in 0 PULSE(0 1 0 1p)\nR1 in a 10\nL1 a b 1m\nC1 b 0 1u\n.tran 2u 1m\n");
    let alpha: f64 = 10.0 / (2.0 * 1e-3);
    let w0: f64 = 1.0 / (1e-3f64 * 1e-6).sqrt();
    let wd = (w0 * w0 - alpha * alpha).sqrt();
    let vc = |t: f64| 1.0 - (-alpha * t).exp() * ((wd * t).cos() + alpha / wd * (wd * t).sin());
    let e = max_err(&w, "v(b)", vc);
    // Overshoot is ~60 %; 1 % of the 1 V step.
    assert!(e < 1e-2, "max error {e}");
}

#[test]
fn lc_tank_keeps_amplitude_with_trapezoidal() {
    let w = tran("lc\nL1 a 0 1m\nC1 a 0 1u ic=1\n.tran 1u 2m uic\n");
    let w0 = 1.0 / (1e-3f64 * 1e-6).sqrt();
    let e = max_err(&w, "v(a)", |t| (w0 * t).cos());
    assert!(e < 2e-2, "max error {e}");
}

#[test]
fn diode_follows_shockley_with_series_resistance() {
    for (i, rs) in [(1e-6f64, 0.0), (1e-3, 0.0), (10e-3, 5.0)] {
        let p = op(&format!(
            "d\nI1 0 a {i}\nD1 a 0 dx\n.model dx d is=1e-14 n=1.5 rs={rs}\n.op\n"
        ));
        let expect = 1.5 * VT * (i / 1e-14 + 1.0).ln() + i * rs;
        let v = p.get("v(a)").unwrap();
        // Newton stops within reltol (1e-3 of the junction voltage).
        assert!((v - expect).abs() < 1e-4, "I={i}: {v} vs {expect}");
    }
}

#[test]
fn diode_reverse_and_breakdown() {
    let p = op("d\nV1 a 0 -5\nD1 a 0 dx\n.model dx d is=1e-14\n.op\n");
    assert!(
        (p.get("i(v1)").unwrap() - 1e-14).abs() < 1e-14 * 0.01 + 5e-12 * 1.01,
        "{:?}",
        p.get("i(v1)")
    );
    // Zener: 10 mA into a 5.1 V breakdown diode sits a little above BV.
    let p = op("z\nI1 0 k 10m\nD1 0 k dz\n.model dz d is=1e-14 bv=5.1 ibv=1m\n.op\n");
    let v = p.get("v(k)").unwrap();
    let expect = 5.1 + VT * (10e-3f64 / 1e-3).ln();
    assert!((v - expect).abs() < 5e-3, "{v} vs {expect}");
}

#[test]
fn bjt_forward_active_bias() {
    // Ib forced, VCE fixed: Ic = BF * Ib, Vbe = Vt ln(Ic / IS).
    let p = op("q\nI1 0 b 10u\nVc c 0 5\nQ1 c b 0 qn\n.model qn npn is=1e-15 bf=100\n.op\n");
    let ic = -p.get("i(vc)").unwrap();
    assert!((ic - 1e-3).abs() < 1e-3 * 1e-4, "ic {ic}");
    let vbe = p.get("v(b)").unwrap();
    // Ib = IS/BF (exp(vbe/vt) - 1) + gmin*vbe terms (negligible)
    let expect = VT * (1e-3f64 / 1e-15 + 1.0).ln();
    assert!((vbe - expect).abs() < 1e-5, "{vbe} vs {expect}");
}

#[test]
fn mosfet_square_law_regions() {
    // KP = 20 µA/V², W/L = 10, VTO = 1, lambda = 0.02.
    let model = ".model nm nmos vto=1 kp=20u lambda=0.02\n";
    let beta = 20e-6 * 10.0;
    let sat = op(&format!(
        "m\nVg g 0 3\nVd d 0 5\nM1 d g 0 0 nm w=10u l=1u\n{model}.op\n"
    ));
    let id = -sat.get("i(vd)").unwrap();
    let expect = beta / 2.0 * 4.0 * (1.0 + 0.02 * 5.0);
    assert!((id - expect).abs() < expect * 1e-6, "sat {id} vs {expect}");
    let lin = op(&format!(
        "m\nVg g 0 3\nVd d 0 0.5\nM1 d g 0 0 nm w=10u l=1u\n{model}.op\n"
    ));
    let id = -lin.get("i(vd)").unwrap();
    let expect = beta * (2.0 - 0.25) * 0.5 * (1.0 + 0.02 * 0.5);
    assert!((id - expect).abs() < expect * 1e-6, "lin {id} vs {expect}");
    let off = op(&format!(
        "m\nVg g 0 0.5\nVd d 0 5\nM1 d g 0 0 nm w=10u l=1u\n{model}.op\n"
    ));
    assert!(off.get("i(vd)").unwrap().abs() < 1e-9);
}

#[test]
fn pmos_mirrors_nmos() {
    let p = op("m\nVs s 0 5\nVg g 0 2\nM1 0 g s s pm w=10u l=1u\n.model pm pmos vto=-1 kp=20u\n.op\n");
    let id = -p.get("i(vs)").unwrap();
    let expect = 20e-6 * 10.0 / 2.0 * 4.0;
    assert!((id - expect).abs() < expect * 1e-6, "{id} vs {expect}");
}

#[test]
fn sine_and_pwl_sources_are_exact_on_a_resistor() {
    let w = tran("s\nV1 a 0 SIN(1 2 1k 0.2m)\nR1 a 0 1k\nV2 b 0 PWL(0 0 1m 1 2m 1 3m -1)\nR2 b 0 1k\n.tran 10u 4m\n");
    let e = max_err(&w, "v(a)", |t| {
        if t < 0.2e-3 {
            1.0
        } else {
            1.0 + 2.0 * (2.0 * std::f64::consts::PI * 1e3 * (t - 0.2e-3)).sin()
        }
    });
    assert!(e < 1e-9, "sin error {e}");
    // PWL corners are breakpoints: they appear exactly in the time axis.
    for corner in [1e-3, 2e-3, 3e-3] {
        assert!(
            w.x.iter().any(|&t| (t - corner).abs() < 1e-15),
            "missing breakpoint {corner}"
        );
    }
    assert!((w.at("v(b)", 2.5e-3).unwrap() - 0.0).abs() < 1e-12);
    assert!((w.at("v(b)", 4e-3).unwrap() + 1.0).abs() < 1e-12);
}

#[test]
fn pulse_edges_are_breakpoints() {
    let w = tran("p\nV1 a 0 PULSE(0 5 1u 10n 20n 2u 5u)\nR1 a 0 1k\n.tran 100n 12u\n");
    for corner in [1e-6, 1.01e-6, 3.01e-6, 3.03e-6, 6e-6, 6.01e-6] {
        assert!(
            w.x.iter().any(|&t| (t - corner).abs() < 1e-14),
            "missing breakpoint {corner}"
        );
    }
    assert_eq!(w.at("v(a)", 2e-6).unwrap(), 5.0);
}

#[test]
fn half_wave_rectifier_peak() {
    let w = tran(
        "r\nV1 in 0 SIN(0 10 50)\nD1 in out dx\nR1 out 0 1k\nC1 out 0 100u\n.model dx d is=1e-14\n.tran 0.1m 60m\n",
    );
    let peak = w.signal("v(out)").unwrap().iter().copied().fold(f64::MIN, f64::max);
    // Peak is 10 V minus a diode drop of roughly 0.7 V.
    assert!(peak > 9.0 && peak < 9.5, "peak {peak}");
}

#[test]
fn dc_sweep_matches_operating_points() {
    let src = "d\nV1 a 0 0\nR1 a b 100\nD1 b 0 dx\n.model dx d is=1e-14\n.dc V1 0 2 0.1\n";
    let mut b = build(src);
    let s = analysis::dc_sweep(&mut b.circuit, &b.options, "v1", 0.0, 2.0, 0.1).unwrap();
    assert_eq!(s.x.len(), 21);
    let vb = s.signal("v(b)").unwrap();
    assert!(vb.windows(2).all(|w| w[1] >= w[0]));
    let single = op("d\nV1 a 0 1.5\nR1 a b 100\nD1 b 0 dx\n.model dx d is=1e-14\n.op\n");
    assert!((s.at("v(b)", 1.5).unwrap() - single.get("v(b)").unwrap()).abs() < 1e-6);
}

#[test]
fn continuation_rescues_a_starved_newton() {
    // A diode string from a stiff 50 V source with only 2 plain Newton
    // iterations allowed: the solver must fall back to a homotopy.
    let mut src = String::from("hard\nV1 n0 0 50\nR1 n0 n1 1\n");
    for k in 1..=8 {
        src.push_str(&format!("D{k} n{k} n{} dx\n", k + 1));
    }
    src.push_str("D9 n9 0 dx\n.model dx d is=1e-16 rs=0.1\n.options itl1=2\n.op\n");
    let p = op(&src);
    assert_ne!(p.method, OpMethod::Newton);
    let v1 = p.get("v(n1)").unwrap();
    assert!(v1 > 9.0 * 0.6 && v1 < 50.0, "{v1}");
}

#[test]
fn singular_circuit_is_reported() {
    let mut b = build("s\nV1 a 0 1\nV2 a 0 2\n.op\n");
    let e = analysis::operating_point(&mut b.circuit, &b.options).unwrap_err();
    assert!(
        matches!(e, AnalysisError::Singular(_) | AnalysisError::NoConvergence(_)),
        "{e}"
    );
}

#[test]
fn build_errors_are_clear() {
    let deck = parse_str("t\nR1 a 0 0\n.op\n").unwrap();
    assert!(
        netlist::build(&deck)
            .unwrap_err()
            .to_string()
            .contains("zero resistance")
    );
    let deck = parse_str("t\nD1 a 0 nomodel\n.op\n").unwrap();
    assert!(netlist::build(&deck).unwrap_err().to_string().contains("unknown model"));
    let deck = parse_str("t\nF1 a 0 vx 2\nR1 a 0 1\n.op\n").unwrap();
    assert!(netlist::build(&deck).is_err());
}

fn ac(src: &str) -> of_analog::results::AcResult {
    let mut b = build(src);
    let a = b
        .analyses
        .iter()
        .find(|a| matches!(a, Analysis::Ac { .. }))
        .cloned()
        .expect(".ac");
    let f = Built::ac_frequencies(&a).unwrap();
    analysis::ac_analysis(&mut b.circuit, &b.options, &f).expect("ac")
}

#[test]
fn ac_rc_corner_is_minus_3db_and_minus_45_degrees() {
    let (r, c) = (1e3, 100e-9);
    let fc = 1.0 / (2.0 * std::f64::consts::PI * r * c);
    let a = ac(&format!(
        "t\nV1 in 0 DC 0 AC 1\nR1 in out {r}\nC1 out 0 {c}\n.ac lin 1 {fc} {fc}\n"
    ));
    assert!((a.mag("v(out)").unwrap()[0] - std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
    assert!((a.phase_deg("v(out)").unwrap()[0] + 45.0).abs() < 1e-7);
    // Decade sweep: 10 points per decade plus the end point.
    let a = ac("t\nV1 in 0 AC 2 90\nR1 in out 1k\nC1 out 0 1u\n.ac dec 10 1 1k\n");
    assert_eq!(a.freq.len(), 31);
    // Source phase carries through: at 1 Hz the output is ~2∠90°.
    assert!((a.mag("v(out)").unwrap()[0] - 2.0).abs() < 1e-3);
    assert!((a.phase_deg("v(out)").unwrap()[0] - 90.0).abs() < 0.5);
}

#[test]
fn ac_series_rlc_resonance_and_q() {
    // Voltage across R peaks at 1/(2π√LC) with unity gain.
    let (l, c, r): (f64, f64, f64) = (10e-3, 1e-6, 10.0);
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * (l * c).sqrt());
    let a = ac(&format!(
        "t\nV1 in 0 AC 1\nL1 in a {l}\nC1 a b {c}\nR1 b 0 {r}\n.ac lin 3 {} {}\n",
        f0 * 0.5,
        f0 * 1.5
    ));
    let m = a.mag("v(b)").unwrap();
    assert!((m[1] - 1.0).abs() < 1e-9, "{m:?}");
    let w = 2.0 * std::f64::consts::PI * f0 * 0.5;
    let x = w * l - 1.0 / (w * c);
    assert!((m[0] - r / (r * r + x * x).sqrt()).abs() < 1e-9);
}

#[test]
fn coupled_inductors_transform_by_turns_ratio() {
    // k = 1, L2 = 4 L1 (turns ratio 2): open-circuit secondary doubles.
    // (A tiny source resistance keeps the DC operating point defined.)
    let ratio = |a: &of_analog::results::AcResult| a.mag("v(s)").unwrap()[0] / a.mag("v(p)").unwrap()[0];
    let a = ac("t\nV1 x 0 AC 1\nRS x p 1m\nL1 p 0 1m\nL2 s 0 4m\nK1 L1 L2 1\nRL s 0 1e9\n.ac lin 1 1k 1k\n");
    assert!((ratio(&a) - 2.0).abs() < 1e-6);
    assert!((a.phase_deg("v(s)").unwrap()[0] - a.phase_deg("v(p)").unwrap()[0]).abs() < 1e-6);
    // Negative coupling inverts.
    let a = ac("t\nV1 x 0 AC 1\nRS x p 1m\nL1 p 0 1m\nL2 s 0 1m\nK1 L1 L2 -0.5\nRL s 0 1e9\n.ac lin 1 1k 1k\n");
    assert!((ratio(&a) - 0.5).abs() < 1e-6);
    // Transient: sine on the primary, the secondary follows k·sqrt(L2/L1).
    let w = tran("t\nV1 x 0 SIN(0 1 1k)\nRS x p 1m\nL1 p 0 1m\nL2 s 0 9m\nK1 L1 L2 0.9\nRL s 0 1e9\n.tran 10u 2m\n");
    let vp = w.signal("v(p)").unwrap().to_vec();
    let vs = w.signal("v(s)").unwrap();
    let err = vp.iter().zip(vs).map(|(p, s)| (s - 2.7 * p).abs()).fold(0.0, f64::max);
    assert!(err < 0.01, "{err}");
}

#[test]
fn capacitor_esr_and_esl_set_the_high_frequency_impedance() {
    // Driven by 1 A: v = Z. At the series resonance |Z| = ESR.
    let (c, esl, esr): (f64, f64, f64) = (1e-6, 1e-9, 0.05);
    let f0 = 1.0 / (2.0 * std::f64::consts::PI * (c * esl).sqrt());
    let a = ac(&format!(
        "t\nI1 0 a AC 1\nRDC a 0 1e12\nC1 a 0 {c} esr={esr} esl={esl}\n.ac lin 1 {f0} {f0}\n"
    ));
    assert!((a.mag("v(a)").unwrap()[0] - esr).abs() < 1e-9);
    // Inductor winding resistance appears in DC.
    let p = op("t\nV1 a 0 1\nL1 a 0 1m rser=2\n.op\n");
    assert!((p.get("i(v1)").unwrap() + 0.5).abs() < 1e-12);
}

#[test]
fn resistor_temperature_coefficients() {
    // R(T) = R (1 + tc1 dT + tc2 dT²) with dT = 100 - 27.
    let p = op("t\n.temp 100\nV1 a 0 1\nR1 a 0 1k tc1=1m tc2=10u\n.op\n");
    let dt: f64 = 73.0;
    let r = 1e3 * (1.0 + 1e-3 * dt + 1e-5 * dt * dt);
    assert!((p.get("i(v1)").unwrap() + 1.0 / r).abs() < 1e-12);
}

#[test]
fn switches_follow_control_with_hysteresis() {
    let p = op("t\nVC c 0 3\nV1 a 0 1\nS1 a b c 0 sm\nR1 b 0 1k\n.model sm sw vt=2 vh=0.5 ron=1 roff=1meg\n.op\n");
    assert!((p.get("v(b)").unwrap() - 1000.0 / 1001.0).abs() < 1e-9);
    // Inside the band the initial state is held: `on` keeps it closed.
    let src = "t\nVC c 0 2\nV1 a 0 1\nS1 a b c 0 sm {}\nR1 b 0 1k\n.model sm sw vt=2 vh=0.5 ron=1 roff=1meg\n.op\n";
    let on = op(&src.replace("{}", "on"));
    let off = op(&src.replace("{}", ""));
    assert!(on.get("v(b)").unwrap() > 0.99);
    assert!(off.get("v(b)").unwrap() < 1e-2);
    // Current-controlled switch.
    let p = op("t\nI1 0 x 5m\nVS x 0 0\nV1 a 0 1\nW1 a b VS wm\nR1 b 0 1k\n.model wm csw it=1m ron=1 roff=1meg\n.op\n");
    assert!(p.get("v(b)").unwrap() > 0.99);
}

#[test]
fn led_optical_outputs() {
    // vf/if define IS; 20 mA through the LED gives bright = 1 and the
    // optical power eqe·I·hc/(qλ).
    let p = op("t\nI1 0 a 20m\nD1 a 0 red\n.model red led(vf=2.0 if=20m wavelength=630n eqe=0.2)\n.op\n");
    // (Newton stops within its tolerances, so allow a little slack.)
    assert!((p.get("v(a)").unwrap() - 2.0).abs() < 1e-4);
    assert!((p.get("bright(d1)").unwrap() - 1.0).abs() < 1e-3);
    let popt = 0.2 * 20e-3 * 1.239_841_98e-6 / 630e-9;
    assert!((p.get("popt(d1)").unwrap() - popt).abs() < 1e-3 * popt);
    // Probes also appear in sweeps.
    let mut b = build("t\nV1 x 0 0\nR1 x a 100\nD1 a 0 red\n.model red led(vf=2.0)\n.dc V1 0 5 1\n");
    let w = analysis::dc_sweep(&mut b.circuit, &b.options, "v1", 0.0, 5.0, 1.0).unwrap();
    let bright = w.signal("bright(d1)").unwrap();
    assert!(bright[0].abs() < 1e-6 && bright[5] > 0.5);
}

#[test]
fn nested_dc_sweep_repeats_the_inner_axis() {
    let mut b = build("t\nV1 a 0 0\nV2 b 0 0\nR1 a b 1k\n.dc V1 0 2 1 V2 0 1 0.5\n");
    let Analysis::Dc {
        source,
        start,
        stop,
        step,
        outer,
    } = b.analyses[0].clone()
    else {
        panic!()
    };
    let o = outer.unwrap();
    let inner = analysis::DcAxis {
        source,
        start,
        stop,
        step,
    };
    let outer = analysis::DcAxis {
        source: o.source,
        start: o.start,
        stop: o.stop,
        step: o.step,
    };
    let w = analysis::dc_sweep_nested(&mut b.circuit, &b.options, &inner, Some(&outer)).unwrap();
    assert_eq!(w.x, vec![0.0, 1.0, 2.0, 0.0, 1.0, 2.0, 0.0, 1.0, 2.0]);
    let i = w.signal("i(v1)").unwrap();
    let v2 = w.signal("v2").unwrap();
    for k in 0..9 {
        assert!((i[k] + (w.x[k] - v2[k]) / 1e3).abs() < 1e-12);
    }
}

#[test]
fn diode_forward_voltage_tracks_temperature() {
    let vf = |t: f64| {
        op(&format!(
            "t\n.temp {t}\nI1 0 a 1m\nD1 a 0 dm\n.model dm d is=1e-14\n.op\n"
        ))
        .get("v(a)")
        .unwrap()
    };
    let slope = (vf(77.0) - vf(27.0)) / 50.0;
    assert!(slope < -1.5e-3 && slope > -2.5e-3, "{slope}");
}

#[test]
fn param_expressions_reach_the_circuit() {
    let p = op("t\n.param r1=2k r2={r1/2} v={3*2}\nV1 a 0 {v}\nR1 a b {r1}\nR2 b 0 {r2}\n.op\n");
    assert!((p.get("v(b)").unwrap() - 2.0).abs() < 1e-9);
}

/// A cold `uic` start puts the whole supply step on forward-biased
/// junctions at t = 0 (as a bench supply switched on into a diode-steered
/// RC and an LED). Newton from all-zero must still converge.
#[test]
fn cold_uic_supply_step_into_forward_diodes_converges() {
    let w = tran(
        "cold\nV1 vcc 0 5\nR1 vcc a 1k\nD1 a cap dsig\nC1 cap 0 100n\nR2 vcc l 220\nD2 l 0 dled\n\
         .model dsig d(is=4n n=1.9 rs=0.5 cjo=4p tt=3n)\n.model dled d(is=1e-18 n=2 rs=4 cjo=20p)\n\
         .tran 1u 2m uic\n",
    );
    let cap = w.signal("v(cap)").unwrap();
    let led = w.signal("v(l)").unwrap();
    // C1 charges through R1 and D1 towards 5 V less the diode drop.
    let end = *cap.last().unwrap();
    assert!((4.3..4.7).contains(&end), "v(cap) at 2 ms = {end}");
    // The LED sits at its forward voltage from the first steps.
    let v_led = *led.last().unwrap();
    assert!((1.6..2.2).contains(&v_led), "v(l) = {v_led}");
}
