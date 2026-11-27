//! Small-signal AC analysis.
//!
//! The circuit is linearised about its DC operating point. Every device is
//! loaded twice in a "small-signal transient" mode with pseudo-integration
//! coefficients `ag0 = 0` and `ag0 = 1`: the first assembly is the
//! conductance matrix `G`, and the difference is the capacitance matrix
//! `C` (charge and flux derivatives). Each frequency then solves
//! `(G + jωC) x = b` with the cached symbolic LU and the sources' AC
//! magnitudes and phases in `b`. Devices therefore need no AC-specific
//! code; BJT excess phase (PTF) is transient-only and ignored here.

use crate::circuit::{Circuit, LoadParams, Mode, TranSpec};
use crate::integrate::{Integrator, States};
use crate::matrix::LinearError;
use crate::results::{AcResult, output_vars};

use super::dc::solve_op;
use super::newton::singular_error;
use super::{AnalysisError, Options};

/// Run an AC analysis at `freqs` (Hz).
pub fn ac_analysis(ckt: &mut Circuit, opts: &Options, freqs: &[f64]) -> Result<AcResult, AnalysisError> {
    ckt.finalize()?;
    if freqs.is_empty() || freqs.iter().any(|f| !f.is_finite() || *f < 0.0) {
        return Err(AnalysisError::Invalid(
            ".ac needs finite, non-negative frequencies".into(),
        ));
    }
    let n = ckt.unknowns() + 1;
    let mut states = States::new(ckt.n_states);
    let mut x = Vec::new();
    solve_op(ckt, &mut x, &mut states, opts, None, false)?;
    let mut rhs = vec![0.0; n];
    let mut lp = LoadParams {
        mode: Mode::TranInit,
        init_junctions: false,
        src_factor: 1.0,
        gmin: opts.gmin,
        diag_gmin: 0.0,
        time: 0.0,
        tran: TranSpec::default(),
        tran_op: false,
        small_signal: true,
        freeze_switches: false,
    };
    ckt.load(&x, &mut rhs, &lp, &Integrator::default(), &mut states);
    states.seed();
    lp.mode = Mode::Tran;
    let mut assemble = |ckt: &mut Circuit, ag0: f64| {
        let mut st = states.clone();
        ckt.load(&x, &mut rhs, &lp, &Integrator::small_signal(ag0), &mut st);
        ckt.system.as_ref().expect("finalized").values().to_vec()
    };
    let g = assemble(ckt, 0.0);
    let gc = assemble(ckt, 1.0);
    let c: Vec<f64> = gc.iter().zip(&g).map(|(a, b)| a - b).collect();

    let mut b_re = vec![0.0; n];
    let mut b_im = vec![0.0; n];
    for d in ckt.devices() {
        d.ac_stimulus(&mut b_re, &mut b_im);
    }
    let vars = output_vars(ckt);
    let mut out = AcResult {
        freq: freqs.to_vec(),
        names: vars.iter().map(|&i| ckt.vars()[i].name.clone()).collect(),
        re: vec![Vec::with_capacity(freqs.len()); vars.len()],
        im: vec![Vec::with_capacity(freqs.len()); vars.len()],
    };
    let system = ckt.system.as_ref().expect("finalized");
    for &f in freqs {
        let (mut re, mut im) = (b_re.clone(), b_im.clone());
        let omega = 2.0 * std::f64::consts::PI * f;
        if let Err(LinearError::Singular(v)) = system.solve_complex(&g, &c, omega, &mut re, &mut im) {
            return Err(singular_error(ckt, v));
        }
        for (k, &i) in vars.iter().enumerate() {
            out.re[k].push(re[i]);
            out.im[k].push(im[i]);
        }
    }
    Ok(out)
}
