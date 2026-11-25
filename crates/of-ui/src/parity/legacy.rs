//! The hand-wired demos behind [`DemoRun`], for recording the goldens.

use of_components::BounceSettings;

use super::{Demo, DemoRun, Raw, Step};
use crate::adc_pwm::{PwmKind, PwmSim};
use crate::blink::Blink;
use crate::button::{ButtonKind, ButtonSim};
use crate::supply::SupplySettings;

fn err(e: impl std::fmt::Display) -> String {
    e.to_string()
}

/// Build the hand-wired simulation for `demo`, with the ESP32-C3's supply
/// current off as in every parity run, and start its edge log.
pub(crate) fn start(demo: Demo) -> Result<Box<dyn DemoRun>, String> {
    Ok(match demo {
        Demo::Blink => {
            let mut s = Blink::with_supply(SupplySettings::default(), 0.0).map_err(err)?;
            s.parity_start();
            Box::new(s)
        }
        Demo::Dimmer | Demo::Fade => {
            let kind = if demo == Demo::Dimmer {
                PwmKind::Dimmer
            } else {
                PwmKind::Fade
            };
            let mut s = PwmSim::with_supply(kind, 0.5, SupplySettings::default(), 0.0).map_err(err)?;
            s.parity_start();
            Box::new(s)
        }
        Demo::Button | Demo::ButtonIrq => {
            let kind = if demo == Demo::Button {
                ButtonKind::Poll
            } else {
                ButtonKind::Irq
            };
            let mut s = ButtonSim::with_supply(kind, SupplySettings::default(), 0.0).map_err(err)?;
            s.parity_start();
            Box::new(s)
        }
    })
}

impl DemoRun for Blink {
    fn step(&mut self, step: Step) -> Result<(), String> {
        match step {
            Step::RunTo(t) => self.advance(t).map_err(err),
            other => Err(format!("blink has no {other:?}")),
        }
    }

    fn raw(&mut self) -> Raw {
        self.parity_raw()
    }
}

impl DemoRun for PwmSim {
    fn step(&mut self, step: Step) -> Result<(), String> {
        match step {
            Step::RunTo(t) => self.advance(t).map_err(err),
            Step::Position(p) => self.set_position(p).map_err(err),
            other => Err(format!("PWM demo has no {other:?}")),
        }
    }

    fn raw(&mut self) -> Raw {
        self.parity_raw()
    }
}

impl DemoRun for ButtonSim {
    fn step(&mut self, step: Step) -> Result<(), String> {
        match step {
            Step::RunTo(t) => self.advance(t).map_err(err),
            Step::Press(closed) => self.set_closed(closed).map_err(err),
            Step::Bounce(enabled) => {
                self.set_bounce(BounceSettings {
                    enabled,
                    ..self.bounce()
                });
                Ok(())
            }
            other => Err(format!("button demo has no {other:?}")),
        }
    }

    fn raw(&mut self) -> Raw {
        self.parity_raw()
    }
}
