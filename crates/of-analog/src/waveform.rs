//! Independent source waveforms (DC, PULSE, SIN, EXP, PWL) with SPICE
//! defaults and breakpoints.

use std::f64::consts::PI;

use of_netlist::spice::{WaveKind, Waveform};

use crate::circuit::TranSpec;

/// A source value as a function of time.
#[derive(Debug, Clone, PartialEq)]
pub enum Wave {
    /// Constant.
    Dc(f64),
    /// `PULSE(v1 v2 td tr tf pw per)`; zero/absent times take SPICE defaults.
    Pulse {
        /// Initial value.
        v1: f64,
        /// Pulsed value.
        v2: f64,
        /// Delay.
        td: f64,
        /// Rise time (0 = tstep).
        tr: f64,
        /// Fall time (0 = tstep).
        tf: f64,
        /// Pulse width (0 = tstop).
        pw: f64,
        /// Period (0 = tstop).
        per: f64,
    },
    /// `SIN(vo va freq td theta phase)`.
    Sin {
        /// Offset.
        vo: f64,
        /// Amplitude.
        va: f64,
        /// Frequency in Hz (0 = 1/tstop).
        freq: f64,
        /// Delay.
        td: f64,
        /// Damping factor (1/s).
        theta: f64,
        /// Phase in degrees.
        phase: f64,
    },
    /// `EXP(v1 v2 td1 tau1 td2 tau2)`.
    Exp {
        /// Initial value.
        v1: f64,
        /// Target value.
        v2: f64,
        /// Rise delay.
        td1: f64,
        /// Rise time constant (0 = tstep).
        tau1: f64,
        /// Fall delay (0 = td1 + tstep).
        td2: f64,
        /// Fall time constant (0 = tstep).
        tau2: f64,
    },
    /// Piecewise linear `(time, value)` points.
    Pwl(Vec<(f64, f64)>),
}

impl Wave {
    /// Convert a parsed waveform card.
    pub fn from_spec(w: &Waveform) -> Result<Self, String> {
        let a = |i: usize| w.args.get(i).copied().unwrap_or(0.0);
        Ok(match w.kind {
            WaveKind::Pulse => {
                if w.args.len() < 2 {
                    return Err("PULSE needs at least v1 and v2".into());
                }
                Wave::Pulse {
                    v1: a(0),
                    v2: a(1),
                    td: a(2),
                    tr: a(3),
                    tf: a(4),
                    pw: a(5),
                    per: a(6),
                }
            }
            WaveKind::Sin => {
                if w.args.len() < 2 {
                    return Err("SIN needs at least vo and va".into());
                }
                Wave::Sin {
                    vo: a(0),
                    va: a(1),
                    freq: a(2),
                    td: a(3),
                    theta: a(4),
                    phase: a(5),
                }
            }
            WaveKind::Exp => {
                if w.args.len() < 2 {
                    return Err("EXP needs at least v1 and v2".into());
                }
                Wave::Exp {
                    v1: a(0),
                    v2: a(1),
                    td1: a(2),
                    tau1: a(3),
                    td2: a(4),
                    tau2: a(5),
                }
            }
            WaveKind::Pwl => {
                if w.args.len() < 2 || !w.args.len().is_multiple_of(2) {
                    return Err("PWL needs time/value pairs".into());
                }
                let pts: Vec<(f64, f64)> = w.args.chunks(2).map(|c| (c[0], c[1])).collect();
                if pts.windows(2).any(|p| p[1].0 < p[0].0) {
                    return Err("PWL times must not decrease".into());
                }
                Wave::Pwl(pts)
            }
        })
    }

    fn pulse_times(&self, tran: &TranSpec) -> Option<(f64, f64, f64, f64, f64)> {
        if let Wave::Pulse {
            td, tr, tf, pw, per, ..
        } = *self
        {
            let tr = if tr > 0.0 { tr } else { tran.tstep };
            let tf = if tf > 0.0 { tf } else { tran.tstep };
            let pw = if pw > 0.0 { pw } else { tran.tstop };
            let per = if per > 0.0 { per } else { tran.tstop };
            Some((td, tr, tf, pw, per))
        } else {
            None
        }
    }

    /// Value at time `t`.
    pub fn value(&self, t: f64, tran: &TranSpec) -> f64 {
        match self {
            Wave::Dc(v) => *v,
            Wave::Pulse { v1, v2, .. } => {
                let (td, tr, tf, pw, per) = self.pulse_times(tran).unwrap_or_default();
                if t <= td {
                    return *v1;
                }
                let mut tt = t - td;
                if per > 0.0 && tt >= per {
                    tt -= (tt / per).floor() * per;
                }
                if tt < tr {
                    v1 + (v2 - v1) * tt / tr
                } else if tt <= tr + pw {
                    *v2
                } else if tt < tr + pw + tf {
                    v2 + (v1 - v2) * (tt - tr - pw) / tf
                } else {
                    *v1
                }
            }
            Wave::Sin {
                vo,
                va,
                freq,
                td,
                theta,
                phase,
            } => {
                let f = if *freq > 0.0 { *freq } else { 1.0 / tran.tstop };
                let ph = phase * PI / 180.0;
                if t <= *td {
                    vo + va * ph.sin()
                } else {
                    let tt = t - td;
                    vo + va * (-tt * theta).exp() * (2.0 * PI * f * tt + ph).sin()
                }
            }
            Wave::Exp {
                v1,
                v2,
                td1,
                tau1,
                td2,
                tau2,
            } => {
                let tau1 = if *tau1 > 0.0 { *tau1 } else { tran.tstep };
                let tau2 = if *tau2 > 0.0 { *tau2 } else { tran.tstep };
                let td2 = if *td2 > 0.0 { *td2 } else { td1 + tran.tstep };
                let mut v = *v1;
                if t > *td1 {
                    v += (v2 - v1) * (1.0 - (-(t - td1) / tau1).exp());
                }
                if t > td2 {
                    v += (v1 - v2) * (1.0 - (-(t - td2) / tau2).exp());
                }
                v
            }
            Wave::Pwl(pts) => {
                let (t0, v0) = pts[0];
                if t <= t0 {
                    return v0;
                }
                for w in pts.windows(2) {
                    let ((ta, va), (tb, vb)) = (w[0], w[1]);
                    if t <= tb {
                        return if tb > ta {
                            va + (vb - va) * (t - ta) / (tb - ta)
                        } else {
                            vb
                        };
                    }
                }
                pts[pts.len() - 1].1
            }
        }
    }

    /// Whether the waveform is actively ramping or oscillating at `t`.
    ///
    /// Breakpoints alone are not enough: a PWL or PULSE rise is linear
    /// *between* corners, so a DC hold would freeze the mid-ramp value.
    pub fn is_changing(&self, t: f64, tran: &TranSpec) -> bool {
        let eps = 1e-15 * tran.tstop.max(1e-12);
        match self {
            Wave::Dc(_) => false,
            Wave::Pulse { v1, v2, .. } => {
                if (v1 - v2).abs() <= 1e-18 {
                    return false;
                }
                let Some((td, tr, tf, pw, per)) = self.pulse_times(tran) else {
                    return false;
                };
                if t + eps < td {
                    return false;
                }
                let tau = if per > 0.0 { (t - td).rem_euclid(per) } else { t - td };
                (eps < tau && tau < tr - eps) || (tr + pw + eps < tau && tau < tr + pw + tf - eps)
            }
            Wave::Sin { va, td, .. } => *td <= t + eps && va.abs() > 0.0,
            Wave::Exp { v1, v2, td1, .. } => *td1 <= t + eps && (v1 - v2).abs() > 0.0,
            Wave::Pwl(pts) => {
                for w in pts.windows(2) {
                    if t + eps < w[0].0 {
                        return false;
                    }
                    if t <= w[1].0 + eps {
                        return (w[1].1 - w[0].1).abs() > 1e-18 && (w[1].0 - w[0].0) > eps;
                    }
                }
                false
            }
        }
    }

    /// First breakpoint strictly after `t`.
    pub fn next_breakpoint(&self, t: f64, tran: &TranSpec) -> Option<f64> {
        let eps = 1e-15 * tran.tstop.max(1e-12);
        match self {
            Wave::Dc(_) => None,
            Wave::Pulse { .. } => {
                let (td, tr, tf, pw, per) = self.pulse_times(tran)?;
                let corners = [0.0, tr, tr + pw, tr + pw + tf];
                let k0 = if t > td && per > 0.0 {
                    ((t - td) / per).floor().max(0.0)
                } else {
                    0.0
                };
                for k in [k0, k0 + 1.0] {
                    for c in corners {
                        let b = td + k * per + c;
                        if b > t + eps {
                            return Some(b);
                        }
                    }
                }
                None
            }
            Wave::Sin { td, .. } => (*td > t + eps).then_some(*td),
            Wave::Exp { td1, td2, .. } => {
                let td2 = if *td2 > 0.0 { *td2 } else { td1 + tran.tstep };
                [*td1, td2].into_iter().find(|b| *b > t + eps)
            }
            Wave::Pwl(pts) => pts.iter().map(|p| p.0).find(|b| *b > t + eps),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TR: TranSpec = TranSpec {
        tstep: 1e-9,
        tstop: 1e-3,
    };

    #[test]
    fn pulse_shape_and_breakpoints() {
        let w = Wave::Pulse {
            v1: 0.0,
            v2: 5.0,
            td: 1e-6,
            tr: 1e-7,
            tf: 2e-7,
            pw: 1e-6,
            per: 4e-6,
        };
        assert_eq!(w.value(0.5e-6, &TR), 0.0);
        assert!((w.value(1.05e-6, &TR) - 2.5).abs() < 1e-9);
        assert_eq!(w.value(1.5e-6, &TR), 5.0);
        assert!((w.value(2.2e-6, &TR) - 2.5).abs() < 1e-9);
        assert_eq!(w.value(3e-6, &TR), 0.0);
        assert!((w.value(5.05e-6, &TR) - 2.5).abs() < 1e-9); // second period
        let mut t = 0.0;
        let mut bps = Vec::new();
        while let Some(b) = w.next_breakpoint(t, &TR) {
            if b > 8.9e-6 {
                break;
            }
            bps.push(b);
            t = b;
        }
        let expect = [1e-6, 1.1e-6, 2.1e-6, 2.3e-6, 5e-6, 5.1e-6, 6.1e-6, 6.3e-6];
        assert_eq!(bps.len(), expect.len(), "{bps:?}");
        for (a, b) in bps.iter().zip(expect) {
            assert!((a - b).abs() < 1e-15, "{bps:?}");
        }
    }

    #[test]
    fn pwl_ramp_is_changing_between_corners() {
        let w = Wave::Pwl(vec![(0.0, 0.0), (1e-3, 5.0)]);
        assert!(w.is_changing(0.0, &TR));
        assert!(w.is_changing(0.5e-3, &TR));
        assert!(!w.is_changing(1.1e-3, &TR));
        let flat = Wave::Pwl(vec![(0.0, 1.0), (1e-3, 1.0), (2e-3, 2.0)]);
        assert!(!flat.is_changing(0.2e-3, &TR));
        assert!(flat.is_changing(1.5e-3, &TR));
    }

    #[test]
    fn sin_exp_pwl() {
        let s = Wave::Sin {
            vo: 1.0,
            va: 2.0,
            freq: 1e3,
            td: 0.0,
            theta: 0.0,
            phase: 90.0,
        };
        assert!((s.value(0.0, &TR) - 3.0).abs() < 1e-12);
        assert!((s.value(0.5e-3, &TR) + 1.0).abs() < 1e-9);
        let e = Wave::Exp {
            v1: 0.0,
            v2: 1.0,
            td1: 0.0,
            tau1: 1e-3,
            td2: 1.0,
            tau2: 1e-3,
        };
        assert!((e.value(1e-3, &TR) - (1.0 - (-1.0f64).exp())).abs() < 1e-12);
        let p = Wave::Pwl(vec![(0.0, 0.0), (1.0, 2.0), (2.0, 2.0), (3.0, -1.0)]);
        assert_eq!(p.value(-1.0, &TR), 0.0);
        assert_eq!(p.value(0.5, &TR), 1.0);
        assert_eq!(p.value(2.5, &TR), 0.5);
        assert_eq!(p.value(9.0, &TR), -1.0);
        assert_eq!(p.next_breakpoint(1.0, &TR), Some(2.0));
    }
}
