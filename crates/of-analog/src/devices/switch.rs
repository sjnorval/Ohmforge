//! Voltage- and current-controlled switches (SPICE `S` and `W`) with
//! hysteresis: on above `VT + VH` (`IT + IH`), off below `VT - VH`, and
//! otherwise holding the previous state.

use std::collections::BTreeMap;

use super::param;
use crate::circuit::{Device, LoadCtx, Mode, Setup, Var};
use crate::matrix::Slot;

/// Switch model (`.model name sw(...)` or `csw(...)`).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SwitchModel {
    /// Threshold (V for `sw`, A for `csw`).
    pub threshold: f64,
    /// Hysteresis half-width (same unit).
    pub hysteresis: f64,
    /// On resistance (Ω).
    pub ron: f64,
    /// Off resistance (Ω).
    pub roff: f64,
}

impl SwitchModel {
    /// Build from `.model` parameters; `current` selects `csw` names
    /// (IT/IH) rather than `sw` names (VT/VH). Returns ignored names.
    pub fn from_params(current: bool, p: &BTreeMap<String, f64>) -> (Self, Vec<String>) {
        let known: &[&str] = if current {
            &["it", "ih", "ron", "roff"]
        } else {
            &["vt", "vh", "ron", "roff"]
        };
        let unknown = p.keys().filter(|k| !known.contains(&k.as_str())).cloned().collect();
        let (t, h) = if current { ("it", "ih") } else { ("vt", "vh") };
        let m = Self {
            threshold: param(p, &[t], 0.0),
            hysteresis: param(p, &[h], 0.0).abs(),
            ron: param(p, &["ron"], 1.0),
            roff: param(p, &["roff"], 1e12),
        };
        (m, unknown)
    }
}

/// What controls a switch.
#[derive(Debug, Clone, PartialEq)]
pub enum SwitchControl {
    /// Voltage between two nodes (`S`).
    Voltage {
        /// Positive control node.
        cp: Var,
        /// Negative control node.
        cn: Var,
    },
    /// Current through a voltage source or inductor (`W`).
    Current {
        /// Controlling device name.
        source: String,
    },
}

/// A controlled switch between `p` and `n`.
#[derive(Debug, Clone)]
pub struct Switch {
    name: String,
    p: Var,
    n: Var,
    control: SwitchControl,
    ctrl_branch: Var,
    model: SwitchModel,
    /// State used inside DC analyses (last iteration's value).
    dc_on: bool,
    s: [Slot; 4],
    state: usize,
}

impl Switch {
    /// Switch initially `on` (when the control is inside the hysteresis band).
    pub fn new(name: &str, p: Var, n: Var, control: SwitchControl, model: SwitchModel, on: bool) -> Self {
        Self {
            name: name.into(),
            p,
            n,
            control,
            ctrl_branch: 0,
            model,
            dc_on: on,
            s: [Slot::NONE; 4],
            state: 0,
        }
    }

    fn decide(&self, v: f64, hold: bool) -> bool {
        let m = &self.model;
        if v > m.threshold + m.hysteresis {
            true
        } else if v < m.threshold - m.hysteresis {
            false
        } else {
            hold
        }
    }
}

impl Device for Switch {
    fn name(&self) -> &str {
        &self.name
    }
    fn is_nonlinear(&self) -> bool {
        true
    }
    fn setup(&mut self, s: &mut Setup<'_>) {
        if let SwitchControl::Current { source } = &self.control {
            self.ctrl_branch = s.branch_of(source).unwrap_or(0);
        }
        self.s = s.slots2(self.p, self.n);
        self.state = s.states(1);
    }
    fn load(&mut self, cx: &mut LoadCtx<'_>) {
        let v = match self.control {
            SwitchControl::Voltage { cp, cn } => cx.v(cp) - cx.v(cn),
            SwitchControl::Current { .. } => cx.v(self.ctrl_branch),
        };
        let st = self.state;
        let on = if cx.small_signal {
            cx.states.now[st] > 0.5
        } else {
            let hold = match cx.mode {
                Mode::Dc | Mode::TranInit => self.dc_on,
                Mode::Tran => cx.states.prev[st] > 0.5,
            };
            let on = if cx.init_junctions {
                self.dc_on
            } else if cx.freeze_switches && cx.mode == Mode::Tran {
                hold
            } else {
                self.decide(v, hold)
            };
            let previous = if cx.mode == Mode::Dc {
                self.dc_on
            } else {
                cx.states.now[st] > 0.5
            };
            if on != previous {
                cx.limited = true;
            }
            if cx.mode != Mode::Tran {
                self.dc_on = on;
            }
            on
        };
        cx.states.now[st] = if on { 1.0 } else { 0.0 };
        let g = if on {
            1.0 / self.model.ron
        } else {
            1.0 / self.model.roff
        };
        cx.conductance(&self.s, g);
    }
    fn state_crossing(&self, x: &[f64], x_new: &[f64], states: &crate::integrate::States) -> Option<(f64, f64)> {
        let was = *states.prev.get(self.state)? > 0.5;
        let is = *states.now.get(self.state)? > 0.5;
        if was == is {
            return None;
        }
        let control = |x: &[f64]| match self.control {
            SwitchControl::Voltage { cp, cn } => x[cp] - x[cn],
            SwitchControl::Current { .. } => x[self.ctrl_branch],
        };
        let m = &self.model;
        let level = if is {
            m.threshold + m.hysteresis
        } else {
            m.threshold - m.hysteresis
        };
        Some((control(x) - level, control(x_new) - level))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hysteresis_band_holds_state() {
        let m = SwitchModel {
            threshold: 1.0,
            hysteresis: 0.2,
            ron: 1.0,
            roff: 1e6,
        };
        let s = Switch::new("s1", 1, 0, SwitchControl::Voltage { cp: 2, cn: 0 }, m, false);
        assert!(s.decide(1.3, false));
        assert!(!s.decide(0.7, true));
        assert!(s.decide(1.0, true));
        assert!(!s.decide(1.0, false));
    }
}
