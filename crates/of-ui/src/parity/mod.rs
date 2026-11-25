//! Trace parity goldens for the desktop demos (engine E4, #48).
//!
//! Each demo runs a fixed, deterministic script and its observable outputs are
//! written to `goldens/<demo>.trace.txt`: the GPIO edges after the circuit is
//! connected, the UART0 text, and the scope samples (LED and PSU current on
//! the fixed sample grid). The hand-wired demos recorded these files first
//! (E4b-1); since E4d they are blessed from the circuit files, which
//! reproduced them byte for byte when the hand-wired demos were deleted.
//!
//! One accepted exception (Stefan, 25 November 2026; RFC 0003, Determinism):
//! against the goldens, samples off the 50 µs grid (LEDC-edge samples in the
//! fade demo) get 1 % relative (`Tolerance::GOLDEN`), because the goldens are
//! recorded on Linux and other platforms' libm moves them. On the same
//! machine the two engine paths must agree at the strict `Tolerance::PARITY`
//! (exact digital outputs, 1 ns edge times, `max(1 nA, 1 ppm)` analog):
//! - `file`: the shipped `circuits/demos` files built by `of_circuit::build`
//!   from the embedded files, parts and firmware, exactly as the desktop
//!   builds them (`session_sim`), with U1's idle draw forced off;
//! - `session`: the demos placed through the model registry in Rust (E4b-1);
//! - the idle-draw golden (`goldens/blink.idle-draw.trace.txt`): the blink
//!   file as shipped, with the ESP32-C3's 23 mA `quiescent_a` (datasheet
//!   v2.5 Table 5-8, CPU running at 160 MHz). Only the PSU current differs
//!   from `blink.trace.txt`, by that supply current.
//!
//! Run with `OF_BLESS=1` to rewrite the goldens from the circuit files, on
//! Linux only.

mod file;
mod session;
mod trace;

pub(crate) use session::build as registry_session;
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

    /// The desktop demo it is.
    pub(crate) fn kind(self) -> crate::kind::DemoKind {
        use crate::kind::DemoKind;
        match self {
            Demo::Blink => DemoKind::Blink,
            Demo::Dimmer => DemoKind::Dimmer,
            Demo::Fade => DemoKind::Fade,
            Demo::Button => DemoKind::ButtonPoll,
            Demo::ButtonIrq => DemoKind::ButtonIrq,
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

/// Compare `got` with its golden, or rewrite the golden with `OF_BLESS=1`.
fn check_or_bless(demo: Demo, got: &DemoTrace) {
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
        panic!("{} from its circuit file does not match its golden: {e}", demo.name());
    }
}

/// The circuit file as the desktop builds it, with U1's idle draw off: the
/// path the goldens are blessed from.
fn file_trace(demo: Demo) -> Option<DemoTrace> {
    let mut sim = match file::start(demo, file::IdleDraw::Off) {
        Ok(sim) => sim,
        Err(e) if skip_without_rom(&e) => return None,
        Err(e) => panic!("{}: {e}", demo.name()),
    };
    Some(play(sim.as_mut(), &demo.script()).unwrap_or_else(|e| panic!("{}: {e}", demo.name())))
}

fn run_file(demo: Demo) {
    let Some(trace) = file_trace(demo) else { return };
    dump(demo, "file", &trace);
    check_or_bless(demo, &trace);
}

#[test]
fn golden_matches_file_blink() {
    run_file(Demo::Blink);
}

#[test]
fn golden_matches_file_dimmer() {
    run_file(Demo::Dimmer);
}

#[test]
fn golden_matches_file_fade() {
    run_file(Demo::Fade);
}

#[test]
fn golden_matches_file_button() {
    run_file(Demo::Button);
}

#[test]
fn golden_matches_file_button_irq() {
    run_file(Demo::ButtonIrq);
}

#[test]
fn file_traces_are_deterministic() {
    let a = file_trace(Demo::Blink).expect("blink needs no ROM");
    let b = file_trace(Demo::Blink).expect("blink needs no ROM");
    assert_eq!(serialize("blink", &a), serialize("blink", &b));
    assert!(compare_traces(&a, &b, Tolerance::PARITY).is_ok());
}

#[test]
fn every_demo_has_a_golden() {
    for demo in Demo::ALL {
        assert!(demo.golden().is_some(), "missing golden for {}", demo.name());
    }
}

/// The registry-built demo must match the golden and, at the strict
/// tolerance, the circuit file on this machine.
fn run_session(demo: Demo) {
    let mut sim = match session::start(demo) {
        Ok(sim) => sim,
        Err(e) if skip_without_rom(&e) => return,
        Err(e) => panic!("{}: {e}", demo.name()),
    };
    let trace = play(sim.as_mut(), &demo.script()).unwrap_or_else(|e| panic!("{}: {e}", demo.name()));
    dump(demo, "session", &trace);
    let golden = demo
        .golden()
        .unwrap_or_else(|| panic!("no golden for {}; run with OF_BLESS=1", demo.name()));
    if let Err(e) = compare(&golden, &trace, Tolerance::GOLDEN) {
        panic!("{} on Session does not match the golden: {e}", demo.name());
    }
    let file = file_trace(demo).expect("file run");
    if let Err(e) = compare_traces(&file, &trace, Tolerance::PARITY) {
        panic!("{} on Session does not match its circuit file: {e}", demo.name());
    }
}

#[test]
fn session_matches_file_blink() {
    run_session(Demo::Blink);
}

#[test]
fn session_matches_file_dimmer() {
    run_session(Demo::Dimmer);
}

#[test]
fn session_matches_file_fade() {
    run_session(Demo::Fade);
}

#[test]
fn session_matches_file_button() {
    run_session(Demo::Button);
}

#[test]
fn session_matches_file_button_irq() {
    run_session(Demo::ButtonIrq);
}

const IDLE_GOLDEN: &str = "blink.idle-draw";

fn idle_golden_path() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("src/parity/goldens")
        .join(format!("{IDLE_GOLDEN}.trace.txt"))
}

/// The shipped blink file keeps the ESP32-C3's 23 mA supply current. Its
/// trace is a separate, reviewed golden: everything but the PSU current
/// matches the idle-draw-off blink exactly, and the PSU current is higher by
/// `quiescent_a` times the rail over 3.3 V (just under 23 mA, as the rail
/// sits a few mV under 3.3 V).
#[test]
fn shipped_blink_file_matches_the_idle_draw_golden() {
    let mut sim = file::start(Demo::Blink, file::IdleDraw::AsShipped).expect("blink file");
    let trace = play(sim.as_mut(), &Demo::Blink.script()).expect("run");
    let text = serialize(IDLE_GOLDEN, &trace);
    if std::env::var_os("OF_BLESS").is_some() {
        std::fs::write(idle_golden_path(), &text).expect("write golden");
        eprintln!("blessed {}", idle_golden_path().display());
    } else {
        let golden = std::fs::read_to_string(idle_golden_path())
            .unwrap_or_else(|_| panic!("no {IDLE_GOLDEN} golden; run with OF_BLESS=1"));
        if let Err(e) = compare(&golden, &trace, Tolerance::GOLDEN) {
            panic!("the shipped blink file does not match the idle-draw golden: {e}");
        }
    }
    let off = play(
        file::start(Demo::Blink, file::IdleDraw::Off)
            .expect("blink file")
            .as_mut(),
        &Demo::Blink.script(),
    )
    .expect("run");
    assert_eq!(trace.edges, off.edges, "idle draw must not move the GPIO edges");
    assert_eq!(trace.uart, off.uart);
    assert_eq!(trace.samples.len(), off.samples.len());
    for (on, off) in trace.samples.iter().zip(&off.samples) {
        assert_eq!(on.0, off.0, "sample times must not move");
        assert!((on.1 - off.1).abs() < 0.05, "LED current moved by {} mA", on.1 - off.1);
        let extra = on.2 - off.2;
        assert!(
            (22.8..=23.0).contains(&extra),
            "idle draw adds {extra} mA at t = {}",
            on.0
        );
    }
}
