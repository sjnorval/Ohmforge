//! Analysis results and CSV/VCD writers.

use std::io::{self, Write};

use crate::analysis::OpMethod;
use crate::circuit::Circuit;

/// A DC operating point.
#[derive(Debug, Clone, PartialEq)]
pub struct OpPoint {
    /// Signal names (`v(node)`, `i(source)`), internal nodes excluded.
    pub names: Vec<String>,
    /// Values in the same order.
    pub values: Vec<f64>,
    /// Which method converged.
    pub method: OpMethod,
    /// Newton iterations used in total.
    pub iterations: usize,
}

pub(crate) fn output_vars(ckt: &Circuit) -> Vec<usize> {
    ckt.vars()
        .iter()
        .enumerate()
        .skip(1)
        .filter(|(_, v)| !v.internal)
        .map(|(i, _)| i)
        .collect()
}

/// Device probe outputs (e.g. LED optical power) at solution `x`.
pub(crate) fn probes(ckt: &Circuit, x: &[f64]) -> Vec<(String, f64)> {
    let mut out = Vec::new();
    for d in ckt.devices() {
        d.probes(x, &mut out);
    }
    out
}

impl OpPoint {
    pub(crate) fn new(ckt: &Circuit, x: &[f64], method: OpMethod, iterations: usize) -> Self {
        let idx = output_vars(ckt);
        let mut names: Vec<String> = idx.iter().map(|&i| ckt.vars()[i].name.clone()).collect();
        let mut values: Vec<f64> = idx.iter().map(|&i| x[i]).collect();
        for (n, v) in probes(ckt, x) {
            names.push(n);
            values.push(v);
        }
        Self {
            names,
            values,
            method,
            iterations,
        }
    }

    /// Value of a signal by name (case-insensitive), e.g. `v(out)`.
    pub fn get(&self, name: &str) -> Option<f64> {
        let key = name.to_ascii_lowercase();
        self.names.iter().position(|n| *n == key).map(|i| self.values[i])
    }

    /// Write `name,value` lines.
    pub fn write_csv(&self, w: &mut dyn Write) -> io::Result<()> {
        writeln!(w, "signal,value")?;
        for (n, v) in self.names.iter().zip(&self.values) {
            writeln!(w, "{n},{v:.12e}")?;
        }
        Ok(())
    }
}

/// Counters from a transient run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct TranStats {
    /// Accepted time points.
    pub accepted: usize,
    /// Steps rejected for truncation error.
    pub rejected_lte: usize,
    /// Steps rejected because Newton did not converge.
    pub rejected_newton: usize,
    /// Newton iterations in total.
    pub newton_iterations: usize,
    /// Breakpoints hit exactly.
    pub breakpoints: usize,
    /// LU factorisations.
    pub factorisations: u64,
}

/// Signals against a swept variable (time for transient, a source value
/// for DC sweeps).
#[derive(Debug, Clone, PartialEq)]
pub struct Waveforms {
    /// Name of the independent variable (`time` or the swept source).
    pub x_name: String,
    /// Independent variable values.
    pub x: Vec<f64>,
    /// Signal names.
    pub names: Vec<String>,
    /// One column per signal.
    pub columns: Vec<Vec<f64>>,
    /// Transient statistics (zero for sweeps).
    pub stats: TranStats,
    vars: Vec<usize>,
    n_probes: usize,
}

/// A DC sweep result.
pub type Sweep = Waveforms;

impl Waveforms {
    pub(crate) fn new(ckt: &Circuit, x_name: String) -> Self {
        let vars = output_vars(ckt);
        let mut names: Vec<String> = vars.iter().map(|&i| ckt.vars()[i].name.clone()).collect();
        let probe_names = probes(ckt, &vec![0.0; ckt.unknowns() + 1]);
        let n_probes = probe_names.len();
        names.extend(probe_names.into_iter().map(|(n, _)| n));
        Self {
            x_name,
            x: Vec::new(),
            columns: vec![Vec::new(); names.len()],
            names,
            stats: TranStats::default(),
            vars,
            n_probes,
        }
    }

    pub(crate) fn push(&mut self, ckt: &Circuit, xv: f64, sol: &[f64]) {
        self.x.push(xv);
        for (col, &i) in self.columns.iter_mut().zip(&self.vars) {
            col.push(sol[i]);
        }
        if self.n_probes > 0 {
            let base = self.vars.len();
            for (k, (_, v)) in probes(ckt, sol).into_iter().enumerate().take(self.n_probes) {
                self.columns[base + k].push(v);
            }
        }
    }

    /// Add a column computed by the analysis (e.g. the outer source of a
    /// nested DC sweep). Must be called before any point is pushed.
    pub(crate) fn add_column(&mut self, name: String) -> usize {
        self.names.push(name);
        self.columns.push(Vec::new());
        self.columns.len() - 1
    }

    /// Results gathered outside the solver (for example a co-simulation
    /// sampled by the CLI), for the CSV and VCD writers. Every column must
    /// have one value per `x`.
    pub fn from_columns(x_name: impl Into<String>, x: Vec<f64>, names: Vec<String>, columns: Vec<Vec<f64>>) -> Self {
        assert_eq!(names.len(), columns.len(), "one column per name");
        assert!(columns.iter().all(|c| c.len() == x.len()), "one value per x");
        Self {
            x_name: x_name.into(),
            x,
            names,
            columns,
            stats: TranStats::default(),
            vars: Vec::new(),
            n_probes: 0,
        }
    }

    /// A signal's samples by name (case-insensitive).
    pub fn signal(&self, name: &str) -> Option<&[f64]> {
        let key = name.to_ascii_lowercase();
        self.names
            .iter()
            .position(|n| *n == key)
            .map(|i| self.columns[i].as_slice())
    }

    /// Linearly interpolated value of `name` at `x`.
    pub fn at(&self, name: &str, x: f64) -> Option<f64> {
        let y = self.signal(name)?;
        Some(interpolate(&self.x, y, x))
    }

    /// Resample every signal onto a uniform grid with spacing `step`.
    pub fn resample(&self, step: f64) -> Waveforms {
        let mut out = Waveforms {
            x: Vec::new(),
            columns: vec![Vec::new(); self.columns.len()],
            ..self.clone()
        };
        if self.x.is_empty() || step <= 0.0 {
            return out;
        }
        let (x0, x1) = (self.x[0], self.x[self.x.len() - 1]);
        let n = ((x1 - x0) / step + 1e-9).floor() as usize;
        for k in 0..=n {
            let x = x0 + step * k as f64;
            out.x.push(x);
            for (c, src) in out.columns.iter_mut().zip(&self.columns) {
                c.push(interpolate(&self.x, src, x));
            }
        }
        out
    }

    /// Write a CSV table with a header row.
    pub fn write_csv(&self, w: &mut dyn Write) -> io::Result<()> {
        write!(w, "{}", self.x_name)?;
        for n in &self.names {
            write!(w, ",{n}")?;
        }
        writeln!(w)?;
        for (k, x) in self.x.iter().enumerate() {
            write!(w, "{x:.12e}")?;
            for c in &self.columns {
                write!(w, ",{:.12e}", c[k])?;
            }
            writeln!(w)?;
        }
        Ok(())
    }

    /// Write an IEEE 1364 VCD file with `real` variables (time in fs).
    pub fn write_vcd(&self, w: &mut dyn Write) -> io::Result<()> {
        writeln!(w, "$date ohmforge $end")?;
        writeln!(w, "$version ohmforge {} $end", env!("CARGO_PKG_VERSION"))?;
        writeln!(w, "$timescale 1fs $end")?;
        writeln!(w, "$scope module circuit $end")?;
        let ids: Vec<String> = (0..self.names.len()).map(vcd_id).collect();
        for (n, id) in self.names.iter().zip(&ids) {
            let clean: String = n
                .chars()
                .map(|c| if c.is_ascii_alphanumeric() || c == '_' { c } else { '_' })
                .collect();
            writeln!(w, "$var real 64 {id} {} $end", clean.trim_end_matches('_'))?;
        }
        writeln!(w, "$upscope $end")?;
        writeln!(w, "$enddefinitions $end")?;
        let mut last_t = None;
        for (k, x) in self.x.iter().enumerate() {
            let t = (x * 1e15).round() as u64;
            if last_t == Some(t) {
                continue;
            }
            last_t = Some(t);
            writeln!(w, "#{t}")?;
            for (c, id) in self.columns.iter().zip(&ids) {
                writeln!(w, "r{:.10e} {id}", c[k])?;
            }
        }
        Ok(())
    }
}

fn vcd_id(mut i: usize) -> String {
    let mut s = String::new();
    loop {
        s.push((b'!' + (i % 94) as u8) as char);
        i /= 94;
        if i == 0 {
            break s;
        }
    }
}

/// Piecewise-linear interpolation of `(xs, ys)` at `x` (clamped at the ends).
pub fn interpolate(xs: &[f64], ys: &[f64], x: f64) -> f64 {
    if xs.is_empty() {
        return f64::NAN;
    }
    if x <= xs[0] {
        return ys[0];
    }
    let last = xs.len() - 1;
    if x >= xs[last] {
        return ys[last];
    }
    let i = xs.partition_point(|&v| v <= x).max(1);
    let (x0, x1) = (xs[i - 1], xs[i]);
    if x1 == x0 {
        return ys[i];
    }
    ys[i - 1] + (ys[i] - ys[i - 1]) * (x - x0) / (x1 - x0)
}

/// Small-signal (`.ac`) result: complex phasors per frequency.
#[derive(Debug, Clone, PartialEq)]
pub struct AcResult {
    /// Frequencies (Hz).
    pub freq: Vec<f64>,
    /// Signal names (`v(node)`, `i(source)`).
    pub names: Vec<String>,
    /// Real parts, one column per signal.
    pub re: Vec<Vec<f64>>,
    /// Imaginary parts, one column per signal.
    pub im: Vec<Vec<f64>>,
}

impl AcResult {
    fn index(&self, name: &str) -> Option<usize> {
        let key = name.to_ascii_lowercase();
        self.names.iter().position(|n| *n == key)
    }

    /// Magnitudes of a signal.
    pub fn mag(&self, name: &str) -> Option<Vec<f64>> {
        let i = self.index(name)?;
        Some(self.re[i].iter().zip(&self.im[i]).map(|(r, m)| r.hypot(*m)).collect())
    }

    /// Magnitudes in dB (20·log10).
    pub fn db(&self, name: &str) -> Option<Vec<f64>> {
        Some(self.mag(name)?.into_iter().map(|m| 20.0 * m.log10()).collect())
    }

    /// Phases in degrees (-180..180].
    pub fn phase_deg(&self, name: &str) -> Option<Vec<f64>> {
        let i = self.index(name)?;
        Some(
            self.re[i]
                .iter()
                .zip(&self.im[i])
                .map(|(r, m)| m.atan2(*r).to_degrees())
                .collect(),
        )
    }

    /// Write `frequency,mag(sig),ph(sig),...` with phases in degrees.
    pub fn write_csv(&self, w: &mut dyn Write) -> io::Result<()> {
        write!(w, "frequency")?;
        for n in &self.names {
            write!(w, ",mag({n}),ph({n})")?;
        }
        writeln!(w)?;
        for (k, f) in self.freq.iter().enumerate() {
            write!(w, "{f:.12e}")?;
            for i in 0..self.names.len() {
                let (r, m) = (self.re[i][k], self.im[i][k]);
                write!(w, ",{:.12e},{:.12e}", r.hypot(m), m.atan2(r).to_degrees())?;
            }
            writeln!(w)?;
        }
        Ok(())
    }
}
