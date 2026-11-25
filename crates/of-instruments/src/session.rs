//! [`Probeable`] for the engine [`Session`] (E5b, RFC 0004, decision D4: the
//! measurement crate depends on the engine, never the reverse).
//!
//! - `v(net)` and `v(a,b)` read the analog solution; the ground net (`0` or
//!   `gnd`) reads 0 V when the solver does not name it.
//! - `i(R1)` is the current into pin 1 of a two-pin part and `i(U1.3V3)` the
//!   current into the named pin, both from `Component::terminal_currents`, so
//!   any branch reads without per-demo registration.
//! - `r(PSU.volts)` is a named `Component::readings` value.
//! - `d(U1.GPIO8)` has no analog value here ([`ProbeError::NotAnalog`]); the
//!   logic analyser reads digital edges from the component's engine (E5g).

use of_engine::Session;

use crate::service::{ProbeError, Probeable};
use crate::signal::Signal;

/// `v(net)` from the solution; the ground net reads 0 V if the solver does not name it.
fn net_volts(s: &mut Session, net: &str) -> Option<f64> {
    s.value(&format!("v({net})"))
        .or_else(|| (net == "0" || net.eq_ignore_ascii_case("gnd")).then_some(0.0))
}

impl Probeable for Session {
    fn now_secs(&self) -> f64 {
        self.now().as_secs_f64()
    }

    fn sample(&mut self, signal: &Signal) -> Result<f64, ProbeError> {
        let unresolved = || ProbeError::Unresolved(signal.clone());
        match signal {
            Signal::NetVolts(net) => net_volts(self, net).ok_or_else(unresolved),
            Signal::DiffVolts(a, b) => match (net_volts(self, a), net_volts(self, b)) {
                (Some(va), Some(vb)) => Ok(va - vb),
                _ => Err(unresolved()),
            },
            Signal::PinCurrent { refdes, pin } => {
                let info = self
                    .components()
                    .iter()
                    .map(|c| c.component.info())
                    .find(|i| i.refdes == *refdes)
                    .ok_or_else(unresolved)?;
                let index = match pin {
                    None if info.pins.len() == 2 => 0,
                    None => return Err(unresolved()),
                    Some(p) => info.pins.iter().position(|q| q.name == *p).ok_or_else(unresolved)?,
                };
                let currents = self.terminal_currents(refdes).ok_or_else(unresolved)?;
                currents.get(index).copied().ok_or_else(unresolved)
            }
            Signal::Reading { refdes, name } => self
                .readings(refdes)
                .into_iter()
                .find(|(k, _)| k == name)
                .map(|(_, v)| v)
                .ok_or_else(unresolved),
            Signal::Digital { .. } => Err(ProbeError::NotAnalog(signal.clone())),
        }
    }
}
