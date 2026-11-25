//! Trace parity goldens for the desktop demos (engine E4b, #48).
//!
//! Each demo runs a fixed, deterministic script and its observable outputs are
//! written to `goldens/<demo>.trace.txt`: the GPIO edges after the circuit is
//! connected, the UART0 text, and the scope samples (LED and PSU current on
//! the fixed sample grid). The hand-wired demos recorded these files first;
//! the circuit-file demos must match them within the tolerances agreed for
//! E4 (exact digital outputs, 1 ns edge times, `max(1 nA, 1 ppm)` analog).
//!
//! Run with `OF_BLESS=1` to rewrite the goldens from the hand-wired demos.

mod legacy;
mod trace;

pub(crate) use trace::{DemoTrace, Raw, Tolerance, compare, compare_traces, serialize};

use std::path::PathBuf;

/// The scripted demos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Demo {
    Blink,
    Dimmer,
    Fade,
    Button,
    ButtonIrq,
}

impl Demo {
    pub(crate) const ALL: [Demo; 5] = [Demo::Blink, Demo::Dimmer, Demo::Fade, Demo::Button, Demo::ButtonIrq];

    /// File stem (matches `circuits/demos/<name>.circuit.toml`).
    pub(crate) fn name(self) -> &'static str {
        match self {
            Demo::Blink => "blink",
            Demo::Dimmer => "dimmer",
            Demo::Fade => "fade",
            Demo::Button => "button",
            Demo::ButtonIrq => "button-irq",
        }
    }

    /// The fixed script for this demo.
    pub(crate) fn script(self) -> Vec<Step> {
        match self {
            // The first advance jumps straight to its end (the sample clock's
            // first tick is t = 0), so a short first step puts the rest of the
            // run on the 50 us grid.
            Demo::Blink => vec![Step::RunTo(50e-6), Step::RunTo(2.6e-3)],
            // The firmware reads the pot every 5 ms, so each position is held
            // for 10 ms (two readings); the plan's 3 ms showed one reading only.
            Demo::Dimmer => vec![
                Step::RunTo(10e-3),
                Step::Position(0.2),
                Step::RunTo(20e-3),
                Step::Position(0.9),
                Step::RunTo(30e-3),
            ],
            Demo::Fade => vec![Step::RunTo(20e-3)],
            Demo::Button | Demo::ButtonIrq => vec![
                Step::RunTo(2e-3),
                Step::Press(true),
                Step::RunTo(7e-3),
                Step::Press(false),
                Step::RunTo(12e-3),
                Step::Bounce(false),
                Step::RunTo(14e-3),
                Step::Press(true),
                Step::RunTo(19e-3),
                Step::Press(false),
                Step::RunTo(24e-3),
            ],
        }
    }

    fn golden_path(self) -> PathBuf {
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("src/parity/goldens")
            .join(format!("{}.trace.txt", self.name()))
    }

    /// Committed golden text, if present.
    pub(crate) fn golden(self) -> Option<String> {
        std::fs::read_to_string(self.golden_path()).ok()
    }
}

/// One scripted host action. Times are seconds since the circuit connected.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum Step {
    /// Run until this time.
    RunTo(f64),
    /// Set the RV1 wiper position (dimmer).
    Position(f64),
    /// Press (`true`) or release SW1 (button demos).
    Press(bool),
    /// Turn SW1 contact bounce on or off (button demos).
    Bounce(bool),
}

/// A demo that can play a [`Step`] script and report its trace.
pub(crate) trait DemoRun {
    fn step(&mut self, step: Step) -> Result<(), String>;
    fn raw(&mut self) -> Raw;
}

/// Play `script` on `demo` and collect the trace.
pub(crate) fn play(demo: &mut dyn DemoRun, script: &[Step]) -> Result<DemoTrace, String> {
    for &s in script {
        demo.step(s)?;
    }
    Ok(demo.raw().into_trace())
}

/// True when a ROM-dependent run should be skipped (ROM not fetched, and
/// `OF_REQUIRE_ESP_ROM` not set).
pub(crate) fn skip_without_rom(err: &str) -> bool {
    if err.contains("not bundled") {
        assert!(
            std::env::var_os("OF_REQUIRE_ESP_ROM").is_none(),
            "OF_REQUIRE_ESP_ROM is set but the ROM was not found: {err}"
        );
        eprintln!("skipping: {err}");
        return true;
    }
    false
}

/// Write `trace` to `$OF_PARITY_DUMP/<demo>.<kind>.trace.txt` when set.
fn dump(demo: Demo, kind: &str, trace: &DemoTrace) {
    if let Some(dir) = std::env::var_os("OF_PARITY_DUMP") {
        let path = std::path::Path::new(&dir).join(format!("{}.{kind}.trace.txt", demo.name()));
        std::fs::write(path, serialize(demo.name(), trace)).expect("dump");
    }
}

fn check_or_bless(demo: Demo, got: &DemoTrace) {
    dump(demo, "legacy", got);
    let text = serialize(demo.name(), got);
    if std::env::var_os("OF_BLESS").is_some() {
        std::fs::write(demo.golden_path(), &text).expect("write golden");
        eprintln!("blessed {}", demo.golden_path().display());
        return;
    }
    let golden = demo
        .golden()
        .unwrap_or_else(|| panic!("no golden for {}; run with OF_BLESS=1", demo.name()));
    if let Err(e) = compare(&golden, got, Tolerance::GOLDEN) {
        panic!("{} does not match its golden: {e}", demo.name());
    }
}

fn run_legacy(demo: Demo) {
    let mut sim = match legacy::start(demo) {
        Ok(sim) => sim,
        Err(e) if skip_without_rom(&e) => return,
        Err(e) => panic!("{}: {e}", demo.name()),
    };
    let trace = play(sim.as_mut(), &demo.script()).unwrap_or_else(|e| panic!("{}: {e}", demo.name()));
    check_or_bless(demo, &trace);
}

#[test]
fn golden_matches_legacy_blink() {
    run_legacy(Demo::Blink);
}

#[test]
fn golden_matches_legacy_dimmer() {
    run_legacy(Demo::Dimmer);
}

#[test]
fn golden_matches_legacy_fade() {
    run_legacy(Demo::Fade);
}

#[test]
fn golden_matches_legacy_button() {
    run_legacy(Demo::Button);
}

#[test]
fn golden_matches_legacy_button_irq() {
    run_legacy(Demo::ButtonIrq);
}

#[test]
fn legacy_traces_are_deterministic() {
    let a = play(legacy::start(Demo::Blink).unwrap().as_mut(), &Demo::Blink.script()).unwrap();
    let b = play(legacy::start(Demo::Blink).unwrap().as_mut(), &Demo::Blink.script()).unwrap();
    assert_eq!(serialize("blink", &a), serialize("blink", &b));
    assert!(compare_traces(&a, &b, Tolerance::PARITY).is_ok());
}

#[test]
fn every_demo_has_a_golden() {
    for demo in Demo::ALL {
        assert!(demo.golden().is_some(), "missing golden for {}", demo.name());
    }
}
