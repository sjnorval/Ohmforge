//! The demos built from their circuit files (`circuits/demos`, embedded by
//! `of_circuit::demos`) and run on `Session` with the legacy sampling loop.
//! This is the E4b-2 half of trace parity.

use of_circuit::ParamValue;
use of_circuit::build::build;
use of_circuit::demos;
use of_circuit::fs::load_parts;
use of_engine::Firmware;

use super::{Demo, DemoRun, session};
use crate::adc_pwm::{self, PwmKind};
use crate::button::{self, ButtonKind};

/// The ESP32-C3 idle draw of a run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum IdleDraw {
    /// `quiescent_a = 0`, as the hand-wired demos (parity runs).
    Off,
    /// The shipped file's `quiescent_a` (23 mA on blink; the idle-draw golden).
    AsShipped,
}

/// The pre-booted chip the desktop would use (the same `OnceLock` caches).
fn booted(demo: Demo) -> Result<Option<Firmware>, String> {
    let soc = match demo {
        Demo::Blink => return Ok(None),
        Demo::Dimmer => adc_pwm::booted(PwmKind::Dimmer)?,
        Demo::Fade => adc_pwm::booted(PwmKind::Fade)?,
        Demo::Button => button::booted(ButtonKind::Poll)?,
        Demo::ButtonIrq => button::booted(ButtonKind::Irq)?,
    };
    Ok(Some(Firmware::Booted(Box::new(soc))))
}

/// Build `demo` from its circuit file.
pub(crate) fn start(demo: Demo, idle: IdleDraw) -> Result<Box<dyn DemoRun>, String> {
    let d = demos::demo(demo.name()).ok_or_else(|| format!("no demo file for {}", demo.name()))?;
    let mut file = of_circuit::parse(d.text).map_err(|e| format!("{}: {e}", d.path))?;
    if idle == IdleDraw::Off {
        let u1 = file
            .components
            .iter_mut()
            .find(|c| c.reference() == "U1")
            .ok_or("no U1")?;
        u1.params.insert("quiescent_a".into(), ParamValue::Number(0.0));
    }
    let parts = load_parts(&session::parts_dir())?;
    let res = demos::resolver().with_booted(move |_, _, _| booted(demo));
    let built = build(d.text, &file, &parts, &session::registry(), &res).map_err(|e| format!("{}: {e}", d.path))?;
    session::run(built.session, demo)
}
