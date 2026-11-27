//! DC operating point (with gmin and source stepping) and DC sweeps.

use crate::circuit::{Circuit, LoadParams, Mode, TranSpec};
use crate::integrate::{Integrator, States};
use crate::results::{OpPoint, Sweep};

use super::newton::{NrOutcome, NrParams, newton, singular_error};
use super::{AnalysisError, Options};

/// `tran` is `Some` for the operating point that starts a transient.
fn params(max_iter: usize, init: bool, src: f64, diag: f64, tran: Option<TranSpec>) -> NrParams {
    let load = LoadParams {
        mode: Mode::Dc,
        init_junctions: init,
        src_factor: src,
        gmin: 0.0,
        diag_gmin: diag,
        time: 0.0,
        tran: tran.unwrap_or_default(),
        tran_op: tran.is_some(),
        small_signal: false,
        freeze_switches: false,
    };
    NrParams {
        load,
        max_iter,
        damping: None,
    }
}

/// How the operating point was found.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpMethod {
    /// Plain Newton-Raphson with junction limiting.
    Newton,
    /// Damped Newton (bounded voltage steps).
    DampedNewton,
    /// gmin stepping.
    GminStepping,
    /// Source stepping.
    SourceStepping,
}

/// Solve the DC operating point into `x`, trying progressively more robust
/// methods. Returns the method and total Newton iterations.
pub(crate) fn solve_op(
    ckt: &mut Circuit,
    x: &mut Vec<f64>,
    states: &mut States,
    opts: &Options,
    tran: Option<TranSpec>,
    warm: bool,
) -> Result<(OpMethod, usize), AnalysisError> {
    ckt.finalize()?;
    let n = ckt.unknowns() + 1;
    if x.len() != n {
        *x = vec![0.0; n];
    }
    let integ = Integrator::default();
    let start = x.clone();
    let mut total = 0;

    // 1. Newton-Raphson (from the previous point for sweeps).
    let max1 = if warm { opts.itl2 } else { opts.itl1 };
    match newton(ckt, x, states, &integ, opts, params(max1, !warm, 1.0, 0.0, tran)) {
        NrOutcome::Converged(k) => return Ok((OpMethod::Newton, k)),
        NrOutcome::Singular(v) => return Err(singular_error(ckt, Some(v))),
        NrOutcome::NoConvergence => total += max1,
    }

    // 2. Damped Newton from the initial guess.
    x.clone_from(&start);
    let mut p = params(opts.itl1, true, 1.0, 0.0, tran);
    p.damping = Some(opts.damping_step);
    if let NrOutcome::Converged(k) = newton(ckt, x, states, &integ, opts, p) {
        return Ok((OpMethod::DampedNewton, total + k));
    }
    total += opts.itl1;

    // Continuation steps use the sweep limit `itl2`, as in SPICE.
    // 3. gmin stepping: a large shunt to ground on every node, reduced by
    //    decades (backing off on failure) until it disappears.
    x.iter_mut().for_each(|v| *v = 0.0);
    let mut gdiag = opts.gmin.max(1e-15) * 10f64.powi(opts.gmin_steps as i32);
    let mut factor = 10.0f64;
    let mut init = true;
    let mut good = x.clone();
    let mut ok = false;
    for _ in 0..200 {
        match newton(ckt, x, states, &integ, opts, params(opts.itl2, init, 1.0, gdiag, tran)) {
            NrOutcome::Converged(k) => {
                total += k;
                init = false;
                good.clone_from(x);
                if gdiag <= opts.gmin {
                    // Final solve without the extra shunt.
                    if let NrOutcome::Converged(k) =
                        newton(ckt, x, states, &integ, opts, params(opts.itl2, false, 1.0, 0.0, tran))
                    {
                        total += k;
                        ok = true;
                    }
                    break;
                }
                gdiag /= factor;
            }
            _ => {
                x.clone_from(&good);
                factor = factor.sqrt();
                if factor < 1.000_05 || init {
                    break;
                }
                gdiag *= factor * factor;
                gdiag /= factor;
            }
        }
    }
    if ok {
        return Ok((OpMethod::GminStepping, total));
    }

    // 4. Source stepping: ramp all independent sources from 0 to 100 %.
    x.iter_mut().for_each(|v| *v = 0.0);
    let mut s = 0.0f64;
    let mut ds = 0.1f64;
    let mut good = x.clone();
    let mut first = true;
    while ds > 1e-7 {
        let target = if first { 0.0 } else { (s + ds).min(1.0) };
        match newton(
            ckt,
            x,
            states,
            &integ,
            opts,
            params(opts.itl2, first, target, 0.0, tran),
        ) {
            NrOutcome::Converged(k) => {
                total += k;
                first = false;
                s = target;
                good.clone_from(x);
                if s >= 1.0 {
                    return Ok((OpMethod::SourceStepping, total));
                }
                ds = (ds * 1.5).min(0.25);
            }
            _ => {
                if first {
                    break;
                }
                x.clone_from(&good);
                ds /= 4.0;
            }
        }
    }
    Err(AnalysisError::NoConvergence(
        "Newton, damped Newton, gmin stepping and source stepping all failed".into(),
    ))
}

/// DC operating point.
pub fn operating_point(ckt: &mut Circuit, opts: &Options) -> Result<OpPoint, AnalysisError> {
    ckt.finalize()?;
    let mut states = States::new(ckt.n_states);
    let mut x = Vec::new();
    let (method, iterations) = solve_op(ckt, &mut x, &mut states, opts, None, false)?;
    Ok(OpPoint::new(ckt, &x, method, iterations))
}

/// One DC sweep axis: a source stepped from `start` to `stop` by `step`.
#[derive(Debug, Clone, PartialEq)]
pub struct DcAxis {
    /// Independent source name.
    pub source: String,
    /// First value.
    pub start: f64,
    /// Last value (included when reached by whole steps).
    pub stop: f64,
    /// Increment (sign must match the direction).
    pub step: f64,
}

impl DcAxis {
    fn values(&self) -> Result<Vec<f64>, AnalysisError> {
        let (start, stop, step) = (self.start, self.stop, self.step);
        if step == 0.0 || (stop - start) * step < 0.0 {
            return Err(AnalysisError::Invalid(format!(
                ".dc step {step} cannot go from {start} to {stop}"
            )));
        }
        let count = ((stop - start) / step + 1e-9).floor() as usize + 1;
        Ok((0..count).map(|k| start + step * k as f64).collect())
    }
}

fn set_source(ckt: &mut Circuit, source: &str, v: f64) -> Result<(), AnalysisError> {
    if ckt.device_mut(source).is_none_or(|d| !d.set_dc(v)) {
        return Err(AnalysisError::UnknownSource(source.into()));
    }
    Ok(())
}

/// Sweep the DC value of independent source `source` from `start` to `stop`.
pub fn dc_sweep(
    ckt: &mut Circuit,
    opts: &Options,
    source: &str,
    start: f64,
    stop: f64,
    step: f64,
) -> Result<Sweep, AnalysisError> {
    let inner = DcAxis {
        source: source.into(),
        start,
        stop,
        step,
    };
    dc_sweep_nested(ckt, opts, &inner, None)
}

/// DC sweep of `inner`, repeated for every value of `outer` if given (as
/// SPICE `.dc src1 ... src2 ...`). The result's x axis is the inner
/// source; nested sweeps add a column named after the outer source, and
/// the inner sweep restarts for each outer value.
pub fn dc_sweep_nested(
    ckt: &mut Circuit,
    opts: &Options,
    inner: &DcAxis,
    outer: Option<&DcAxis>,
) -> Result<Sweep, AnalysisError> {
    ckt.finalize()?;
    let inner_values = inner.values()?;
    set_source(ckt, &inner.source, inner.start)?;
    let outer_values = match outer {
        Some(o) => {
            set_source(ckt, &o.source, o.start)?;
            o.values()?
        }
        None => vec![f64::NAN],
    };
    let mut states = States::new(ckt.n_states);
    let mut x = Vec::new();
    let mut sweep = Sweep::new(ckt, inner.source.clone());
    let outer_col = outer.map(|o| sweep.add_column(o.source.clone()));
    let mut first = true;
    for &ov in &outer_values {
        if let Some(o) = outer {
            set_source(ckt, &o.source, ov)?;
        }
        for &v in &inner_values {
            set_source(ckt, &inner.source, v)?;
            if solve_op(ckt, &mut x, &mut states, opts, None, !first).is_err() {
                x.clear();
                solve_op(ckt, &mut x, &mut states, opts, None, false)?;
            }
            first = false;
            sweep.push(ckt, v, &x);
            if let Some(c) = outer_col {
                sweep.columns[c].push(ov);
            }
        }
    }
    Ok(sweep)
}
