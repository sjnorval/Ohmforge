//! Transient analysis with LTE-controlled adaptive steps and breakpoints.

use std::collections::{BTreeMap, VecDeque};

use crate::circuit::{Circuit, LoadParams, Mode, TranSpec};
use crate::integrate::{Integrator, States};
use crate::results::{TranStats, Waveforms};

use super::dc::solve_op;
use super::newton::{NrOutcome, NrParams, newton, singular_error};
use super::{AnalysisError, Options};

/// Transient request (SPICE `.tran tstep tstop [tstart [tmax]] [uic]`).
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TranRequest {
    /// Output step; also bounds the internal step unless `tmax` is set.
    pub tstep: f64,
    /// Stop time.
    pub tstop: f64,
    /// Results before this time are discarded.
    pub tstart: f64,
    /// Largest internal step (0 = `min(tstep, (tstop - tstart) / 50)`).
    pub tmax: f64,
    /// Skip the operating point; start from `.ic` and device `ic=` values.
    pub uic: bool,
    /// Node initial conditions (`.ic v(node)=...`) used with `uic`.
    pub ic: BTreeMap<String, f64>,
}

/// Most steps in a row [`Stepper::rescue`] may take before the run fails.
const MAX_RESCUES: u32 = 32;

fn load_params(mode: Mode, time: f64, tran: TranSpec) -> LoadParams {
    LoadParams {
        mode,
        init_junctions: false,
        src_factor: 1.0,
        gmin: 0.0,
        diag_gmin: 0.0,
        time,
        tran,
        tran_op: false,
        small_signal: false,
        freeze_switches: false,
    }
}

/// An accepted time point: solution and device states.
struct Point {
    t: f64,
    x: Vec<f64>,
    q: Vec<f64>,
}

/// Polynomial extrapolation through the newest `k + 1` history points to
/// time `t` (the Newton starting guess).
fn predict(hist: &VecDeque<Point>, k: usize, t: f64) -> Vec<f64> {
    let pts: Vec<&Point> = hist.iter().rev().take(k + 1).collect();
    let mut out = vec![0.0; pts[0].x.len()];
    for (i, pi) in pts.iter().enumerate() {
        let mut w = 1.0;
        for (j, pj) in pts.iter().enumerate() {
            if i != j {
                w *= (t - pj.t) / (pi.t - pj.t);
            }
        }
        for (o, v) in out.iter_mut().zip(&pi.x) {
            *o += w * v;
        }
    }
    out
}

/// Charge tolerance floor (SPICE `chgtol`).
const CHGTOL: f64 = 1e-14;

/// Largest step allowed by the truncation error of every integrated charge,
/// from the `(order + 1)`-th divided difference of the charge history
/// (the SPICE3 `CKTterr` estimate, in current units).
fn truncation_step(integ: &Integrator, t_new: f64, states: &States, hist: &VecDeque<Point>, opts: &Options) -> f64 {
    let order = integ.order as usize;
    let pts: Vec<(f64, &[f64])> = std::iter::once((t_new, states.now.as_slice()))
        .chain(hist.iter().rev().take(order + 1).map(|p| (p.t, p.q.as_slice())))
        .collect();
    if pts.len() < order + 2 {
        return f64::INFINITY;
    }
    let factor = integ.error_constant();
    let mut best = f64::INFINITY;
    let mut dd = vec![0.0; order + 2];
    for (k, _) in states.charges.iter().enumerate().filter(|(_, c)| **c) {
        for (d, p) in dd.iter_mut().zip(&pts) {
            *d = p.1[k];
        }
        for level in 1..=order + 1 {
            for i in 0..=order + 1 - level {
                dd[i] = (dd[i] - dd[i + 1]) / (pts[i].0 - pts[i + level].0);
            }
        }
        let (q0, q1) = (pts[0].1[k], pts[1].1[k]);
        let (i0, i1) = (pts[0].1[k + 1], pts[1].1[k + 1]);
        let volt_tol = opts.abstol + opts.reltol * i0.abs().max(i1.abs());
        let charge_tol = opts.reltol * q0.abs().max(q1.abs()).max(CHGTOL) / integ.h;
        let tol = volt_tol.max(charge_tol);
        let del = opts.trtol * tol / opts.abstol.max(factor * dd[0].abs());
        let step = if order == 2 { del.sqrt() } else { del };
        best = best.min(step);
    }
    best
}

/// A threshold watch: report when `x[var] - level` changes sign.
///
/// `level` is the constant [`Self::level`] when [`Self::ref_var`] is `None`,
/// otherwise `fraction * x[ref_var]` (so a GPIO V_IH can track a live VDD).
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Crossing {
    pub var: usize,
    pub level: f64,
    pub ref_var: Option<usize>,
    pub fraction: f64,
    pub rising: bool,
    pub falling: bool,
}

impl Crossing {
    fn threshold(&self, x: &[f64]) -> f64 {
        match self.ref_var {
            Some(r) => self.fraction * x[r],
            None => self.level,
        }
    }

    fn signed(&self, x: &[f64]) -> f64 {
        x[self.var] - self.threshold(x)
    }

    fn crossed(&self, x0: &[f64], x1: &[f64]) -> bool {
        let (s0, s1) = (self.signed(x0), self.signed(x1));
        (self.rising && s0 < 0.0 && s1 >= 0.0) || (self.falling && s0 > 0.0 && s1 <= 0.0)
    }
}

/// How an accepted step ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StepEnd {
    /// An ordinary step.
    Free,
    /// Landed on a device breakpoint (source corner) or the horizon.
    Breakpoint,
    /// Landed on the caller's target time.
    Target,
    /// Landed on (just after) a watched threshold crossing.
    Crossing(usize),
}

/// The transient time-stepping state, shared by [`transient`] (one run to
/// `tstop`) and the scheduler engine (advanced slice by slice).
#[derive(Debug)]
pub(crate) struct Stepper {
    tran: TranSpec,
    /// Time of the last accepted point (s).
    pub t: f64,
    h: f64,
    h_prev: f64,
    order: u8,
    hist: VecDeque<Point>,
    /// Solution at `t`.
    pub x: Vec<f64>,
    x_new: Vec<f64>,
    states: States,
    max_step: f64,
    pub h_min: f64,
    bp: f64,
    horizon: f64,
    pub stats: TranStats,
    /// Last accepted step left the solution flat enough that the engine can
    /// jump time without re-solving until a parameter or breakpoint changes.
    pub quiescent: bool,
    /// Steps in a row that only [`Self::rescue`] could take.
    rescues: u32,
}

impl std::fmt::Debug for Point {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Point({})", self.t)
    }
}

impl Stepper {
    /// Operating point (or `uic` start), initial charges and history.
    /// `horizon` caps the breakpoint search (`tstop` of a `.tran`).
    pub fn start(ckt: &mut Circuit, opts: &Options, req: &TranRequest, horizon: f64) -> Result<Self, AnalysisError> {
        ckt.finalize()?;
        let tran = TranSpec {
            tstep: req.tstep,
            tstop: req.tstop,
        };
        let n = ckt.unknowns() + 1;
        let mut states = States::new(ckt.n_states);
        let mut x = vec![0.0; n];
        if req.uic {
            for (node, v) in &req.ic {
                if let Some(var) = ckt.find_node(node) {
                    x[var] = *v;
                }
            }
            for d in ckt.devices() {
                d.initial_condition(&mut x);
            }
        } else {
            solve_op(ckt, &mut x, &mut states, opts, Some(tran), false)?;
        }
        // Initial charges, then seed the history.
        let mut rhs = vec![0.0; n];
        let mut lp = load_params(Mode::TranInit, 0.0, tran);
        lp.gmin = opts.gmin;
        ckt.load(&x, &mut rhs, &lp, &Integrator::default(), &mut states);
        states.seed();

        let span = req.tstop - req.tstart;
        let max_step = if req.tmax > 0.0 {
            req.tmax
        } else {
            req.tstep.min(span / 50.0)
        };
        let h_min = 1e-11 * max_step;
        let h = (req.tstop / 100.0).min(req.tstep) / 10.0;
        let hist = VecDeque::from([Point {
            t: 0.0,
            x: x.clone(),
            q: states.now.clone(),
        }]);
        let mut st = Self {
            tran,
            t: 0.0,
            h,
            h_prev: h,
            order: 1,
            hist,
            x_new: x.clone(),
            x,
            states,
            max_step,
            h_min,
            bp: horizon,
            horizon,
            stats: TranStats::default(),
            // The operating point is a DC solution; `uic` initial
            // conditions usually are not, so the first steps must run.
            quiescent: !req.uic,
            rescues: 0,
        };
        st.bp = st.next_bp(ckt);
        Ok(st)
    }

    fn next_bp(&self, ckt: &Circuit) -> f64 {
        let (t, h_min) = (self.t, self.h_min);
        ckt.devices()
            .filter_map(|d| d.next_breakpoint(t, &self.tran))
            .filter(|&b| b > t + h_min)
            .fold(self.horizon, f64::min)
    }

    /// The circuit changed at the current time (a source or parameter was
    /// set from outside): restart the history like after a breakpoint.
    pub fn discontinuity(&mut self, ckt: &Circuit) {
        self.hist.retain(|p| p.t == self.t);
        if self.hist.is_empty() {
            self.hist.push_back(Point {
                t: self.t,
                x: self.x.clone(),
                q: self.states.prev.clone(),
            });
        }
        self.order = 1;
        self.h = (0.1 * self.h).max(10.0 * self.h_min);
        self.bp = self.next_bp(ckt);
        self.quiescent = false;
    }

    /// The solution is flat and nothing is due before `target`, so the engine
    /// may jump time without taking Newton steps.
    pub fn can_hold(&self, ckt: &Circuit, target: f64) -> bool {
        self.quiescent
            && self.next_bp(ckt) > target - self.h_min
            && !ckt.devices().any(|d| d.is_changing(self.t, &self.tran))
    }

    /// Advance time to `target` keeping the current solution (a DC hold).
    pub fn hold_to(&mut self, target: f64) {
        if target <= self.t {
            return;
        }
        self.t = target;
        // The solution is unchanged, so only the timestamp moves. Avoid
        // cloning `x`/`q` on every quiet kernel slice.
        if let Some(last) = self.hist.back_mut() {
            last.t = target;
        } else {
            self.hist.push_back(Point {
                t: self.t,
                x: self.x.clone(),
                q: self.states.now.clone(),
            });
        }
    }

    /// Take one accepted step, never beyond `target` (which must be after
    /// `t`). With `watches`, a step that crosses a threshold is redone so
    /// that it ends within `resolution` seconds after the crossing.
    /// Last resort before giving up on a step the solver could not shrink
    /// further: damped Newton with more iterations, then a diagonal-gmin
    /// ramp from 1e-2 S down to nothing, each from the predicted point. It
    /// only runs where the step would otherwise fail, so converging runs
    /// are untouched. Leaves the solution in `x_new` on success.
    fn rescue(&mut self, ckt: &mut Circuit, opts: &Options, integ: &Integrator, t_new: f64, x_pred: &[f64]) -> bool {
        let params = |diag_gmin: f64| {
            let mut load = load_params(Mode::Tran, t_new, self.tran);
            load.diag_gmin = diag_gmin;
            // After the plain damped try, hold the switches where the step
            // started: a latch built from switches (a 555's, inside its
            // hysteresis band near VCC = 0) can flip every iteration.
            load.freeze_switches = true;
            NrParams {
                load,
                max_iter: 4 * opts.itl4,
                damping: Some(opts.damping_step),
            }
        };
        self.x_new.copy_from_slice(x_pred);
        self.x_new[0] = 0.0;
        let mut p = params(0.0);
        p.load.freeze_switches = false;
        if let NrOutcome::Converged(k) = newton(ckt, &mut self.x_new, &mut self.states, integ, opts, p) {
            self.stats.newton_iterations += k;
            return true;
        }
        let p = params(0.0);
        self.x_new.copy_from_slice(x_pred);
        self.x_new[0] = 0.0;
        if let NrOutcome::Converged(k) = newton(ckt, &mut self.x_new, &mut self.states, integ, opts, p) {
            self.stats.newton_iterations += k;
            return true;
        }
        self.x_new.copy_from_slice(x_pred);
        self.x_new[0] = 0.0;
        let mut g = 1e-2;
        loop {
            let p = params(g);
            match newton(ckt, &mut self.x_new, &mut self.states, integ, opts, p) {
                NrOutcome::Converged(k) => self.stats.newton_iterations += k,
                _ => break,
            }
            if g == 0.0 {
                return true;
            }
            g = if g / 10.0 < opts.gmin.max(1e-15) { 0.0 } else { g / 10.0 };
        }
        // Source stepping at this time point: the independent sources ramp
        // from zero to their value in adaptive steps, each solve starting
        // from the last; the final solve is the step's own equations.
        self.x_new.fill(0.0);
        let mut good = self.x_new.clone();
        let (mut src, mut ds, mut first) = (0.0f64, 0.1f64, true);
        while first || src < 1.0 {
            let next = if first { 0.0 } else { (src + ds).min(1.0) };
            let mut p = params(0.0);
            p.load.src_factor = next;
            match newton(ckt, &mut self.x_new, &mut self.states, integ, opts, p) {
                NrOutcome::Converged(k) => {
                    self.stats.newton_iterations += k;
                    good.copy_from_slice(&self.x_new);
                    src = next;
                    first = false;
                    ds = (ds * 2.0).min(0.25);
                }
                _ if first => return false,
                _ => {
                    self.x_new.copy_from_slice(&good);
                    ds *= 0.25;
                    if ds < 1e-4 {
                        return false;
                    }
                }
            }
        }
        true
    }

    pub fn step(
        &mut self,
        ckt: &mut Circuit,
        opts: &Options,
        target: f64,
        watches: &[Crossing],
        resolution: f64,
    ) -> Result<StepEnd, AnalysisError> {
        let h_min = self.h_min;
        let mut clip: Option<f64> = None;
        let mut redo = 0;
        let mut sw_redo = 0;
        let h0 = self.h;
        loop {
            let mut h = self.h.min(self.max_step);
            let h_free = h;
            let mut hit = false;
            let bp = self.bp;
            let t = self.t;
            if t + h >= bp - h_min {
                h = bp - t;
                hit = true;
            } else if t + 2.0 * h > bp {
                h = (bp - t) / 2.0;
            }
            // Caller's target or a crossing estimate, whichever is earlier.
            let stop = clip.map_or(target, |c| c.min(target));
            let mut at_target = false;
            if t + h >= stop - h_min && stop < bp - h_min {
                h = stop - t;
                hit = false;
                at_target = true;
            }
            let integ = Integrator::new(opts.method, self.order, h, self.h_prev);
            let t_new = if hit {
                bp
            } else if at_target {
                stop
            } else {
                t + h
            };
            let k_pred = (self.order as usize).min(self.hist.len() - 1);
            let x_pred = predict(&self.hist, k_pred, t_new);
            self.x_new.copy_from_slice(&x_pred);
            self.x_new[0] = 0.0;
            let p = NrParams {
                load: load_params(Mode::Tran, t_new, self.tran),
                max_iter: opts.itl4,
                damping: None,
            };
            match newton(ckt, &mut self.x_new, &mut self.states, &integ, opts, p) {
                NrOutcome::Converged(k) => {
                    self.stats.newton_iterations += k;
                    self.rescues = 0;
                }
                NrOutcome::Singular(v) => return Err(singular_error(ckt, Some(v))),
                NrOutcome::NoConvergence if sw_redo > 0 && sw_redo < 60 => {
                    // Chasing a switch crossing made Newton fail: take the
                    // step without the chase (the old end-of-step flip).
                    self.stats.newton_iterations += opts.itl4;
                    self.stats.rejected_newton += 1;
                    sw_redo = 60;
                    clip = None;
                    self.h = h0;
                    continue;
                }
                NrOutcome::NoConvergence => {
                    self.stats.newton_iterations += opts.itl4;
                    self.stats.rejected_newton += 1;
                    // Bounded: a run of rescued steps at the minimum step
                    // is a crawl, not progress, so it ends in the error.
                    if h / 8.0 < h_min && self.rescues < MAX_RESCUES && self.rescue(ckt, opts, &integ, t_new, &x_pred) {
                        // Damped Newton, a gmin ramp or source stepping
                        // converged where shrinking the step could not.
                        self.rescues += 1;
                    } else {
                        self.h = h / 8.0;
                        self.order = 1;
                        if self.h < h_min {
                            return Err(AnalysisError::TimestepTooSmall {
                                time: t,
                                reason: "Newton did not converge".into(),
                            });
                        }
                        continue;
                    }
                }
            }

            // Charge-based truncation error (SPICE CKTterr), skipped on the
            // first step after a breakpoint where there is no usable history.
            let mut h_lte = f64::INFINITY;
            if self.hist.len() > self.order as usize {
                h_lte = truncation_step(&integ, t_new, &self.states, &self.hist, opts);
                if h_lte < 0.9 * h {
                    self.stats.rejected_lte += 1;
                    self.h = h_lte.max(h / 8.0);
                    self.order = 1.max(self.order.min(self.hist.len() as u8 - 1));
                    if self.h < h_min {
                        return Err(AnalysisError::TimestepTooSmall {
                            time: t,
                            reason: "truncation error above tolerance".into(),
                        });
                    }
                    continue;
                }
            }

            // Threshold crossings: shorten the step to the (linearly
            // interpolated) crossing until it lands within `resolution`.
            let mut fired = None;
            for (k, w) in watches.iter().enumerate() {
                if w.crossed(&self.x, &self.x_new) {
                    let s0 = w.signed(&self.x);
                    let s1 = w.signed(&self.x_new);
                    let tc = t + (t_new - t) * s0 / (s0 - s1);
                    if t_new - tc > resolution && redo < 12 && t_new - t > resolution {
                        let c = clip.map_or(tc, |c: f64| c.min(tc));
                        clip = Some(c.max(t + 0.5 * resolution.min(t_new - t)));
                        fired = Some(usize::MAX);
                        break;
                    }
                    fired.get_or_insert(k);
                }
            }
            // Switch events: a switch that flipped inside the step acted
            // over all of it, so redo the step to end within `sw_res` after
            // the control crossed its threshold (independent of max_step).
            let mut flipped = false;
            if fired != Some(usize::MAX) {
                let sw_res = (1e-4 * self.max_step).max(10.0 * h_min);
                for d in ckt.devices() {
                    let Some((s0, s1)) = d.state_crossing(&self.x, &self.x_new, &self.states) else {
                        continue;
                    };
                    flipped = true;
                    if s0 * s1 < 0.0 && t_new - t > sw_res && sw_redo < 60 {
                        let tc = t + (t_new - t) * s0 / (s0 - s1);
                        if t_new - tc > sw_res {
                            let c = clip.map_or(tc, |c: f64| c.min(tc));
                            clip = Some(c.max(t + 0.5 * sw_res.min(t_new - t)));
                            fired = Some(usize::MAX);
                            sw_redo += 1;
                            break;
                        }
                    }
                }
            }
            if fired == Some(usize::MAX) {
                redo += 1;
                self.h = h;
                continue;
            }

            // Accept.
            let quiet = fired.is_none() && !hit && {
                let dt = (t_new - t).max(self.h_min);
                let mut max_dx: f64 = 0.0;
                for (a, b) in self.x.iter().zip(self.x_new.iter()) {
                    max_dx = max_dx.max((b - a).abs());
                }
                max_dx / dt * self.max_step <= opts.vntol
            };
            self.t = t_new;
            self.states.rotate();
            self.x.copy_from_slice(&self.x_new);
            self.stats.accepted += 1;
            self.quiescent = quiet;
            self.h_prev = h;
            let mut h_next = (2.0 * h).min(h_lte);
            let end = if hit {
                self.stats.breakpoints += 1;
                self.hist.clear();
                self.bp = self.next_bp(ckt);
                // Restart gently after a corner (as SPICE does).
                h_next = (0.1 * h.min(self.bp - self.t)).max(10.0 * h_min);
                StepEnd::Breakpoint
            } else if at_target {
                // A slice end is not a discontinuity: keep the history and
                // carry on with the step the error control wanted.
                h_next = h_free.min(h_lte).max(h);
                StepEnd::Target
            } else {
                StepEnd::Free
            };
            if flipped && !hit {
                // A switch flipped: the circuit is discontinuous here, so
                // restart the history and the step size like at a corner.
                self.hist.clear();
                h_next = (0.1 * h).max(10.0 * h_min);
            }
            self.hist.push_back(Point {
                t: self.t,
                x: self.x.clone(),
                q: self.states.now.clone(),
            });
            while self.hist.len() > 4 {
                self.hist.pop_front();
            }
            // Second order needs three history points for its error estimate.
            self.order = if self.hist.len() >= 3 { 2 } else { 1 };
            self.h = h_next;
            return Ok(match fired {
                Some(k) => StepEnd::Crossing(k),
                None if at_target && clip.is_some_and(|c| c < target) => StepEnd::Free,
                None => end,
            });
        }
    }

    /// Final statistics.
    pub fn finish_stats(&self, ckt: &Circuit) -> TranStats {
        let mut s = self.stats;
        s.factorisations = ckt.system.as_ref().map_or(0, |s| s.stats().factorisations);
        s
    }
}

/// Run a transient analysis.
pub fn transient(ckt: &mut Circuit, opts: &Options, req: &TranRequest) -> Result<Waveforms, AnalysisError> {
    let (tstep, tstop) = (req.tstep, req.tstop);
    if !(tstep > 0.0 && tstop > 0.0 && req.tstart >= 0.0 && req.tstart < tstop) {
        return Err(AnalysisError::Invalid(format!(
            ".tran needs 0 < tstep, 0 <= tstart < tstop (got {tstep}, {tstop}, {})",
            req.tstart
        )));
    }
    let mut st = Stepper::start(ckt, opts, req, tstop)?;
    let mut out = Waveforms::new(ckt, "time".into());
    if req.tstart <= 0.0 {
        out.push(ckt, 0.0, &st.x);
    }
    while st.t < tstop - st.h_min {
        st.step(ckt, opts, f64::INFINITY, &[], 0.0)?;
        if st.t >= req.tstart {
            out.push(ckt, st.t, &st.x);
        }
    }
    out.stats = st.finish_stats(ckt);
    Ok(out)
}
