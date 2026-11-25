//! Trace text format and comparison for the parity goldens.

use std::fmt::Write as _;

use of_core::SimTime;
use of_soc_esp32c3::PinEdge;

/// Raw outputs read from a running demo.
pub(crate) struct Raw {
    /// Edges since logging started (absolute kernel time).
    pub edges: Vec<PinEdge>,
    /// Kernel time when the circuit was connected.
    pub origin: SimTime,
    /// UART0 text so far.
    pub uart: String,
    /// LED current samples `(t, mA)`, `t` relative to `origin`.
    pub led: Vec<(f64, f64)>,
    /// PSU current samples `(t, mA)` at the same times.
    pub psu: Vec<(f64, f64)>,
}

impl Raw {
    pub(crate) fn into_trace(self) -> DemoTrace {
        let edges = self
            .edges
            .iter()
            // Boot-time edges from before the circuit was connected are not part
            // of the circuit trace.
            .filter(|e| e.at >= self.origin)
            .map(|e| (e.at.0 - self.origin.0, e.pin, edge_code(e)))
            .collect();
        let samples = self
            .led
            .iter()
            .zip(self.psu.iter())
            .map(|(&(t, led), &(_, psu))| (t, led, psu))
            .collect();
        DemoTrace {
            edges,
            uart: self.uart,
            samples,
        }
    }
}

/// Output-stage code: `H`, `L` or `Z`, drive strength, then `u` / `d` for pulls.
fn edge_code(e: &PinEdge) -> String {
    let mut s = String::new();
    s.push(match e.out.level {
        Some(true) => 'H',
        Some(false) => 'L',
        None => 'Z',
    });
    let _ = write!(s, "{}", e.out.strength);
    if e.out.pull_up {
        s.push('u');
    }
    if e.out.pull_down {
        s.push('d');
    }
    s
}

/// One demo's observable outputs.
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct DemoTrace {
    /// `(ps since the circuit connected, GPIO, output code)`.
    pub edges: Vec<(u64, u8, String)>,
    /// UART0 text.
    pub uart: String,
    /// `(t seconds, LED mA, PSU mA)`.
    pub samples: Vec<(f64, f64, f64)>,
}

/// Comparison tolerances. Digital outputs (UART text, edge count, order,
/// pins and output codes, sample times) are always exact.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Tolerance {
    /// Edge time tolerance (ps).
    pub edge_ps: u64,
    /// Absolute analog tolerance (mA).
    pub abs_ma: f64,
    /// Relative analog tolerance.
    pub rel: f64,
    /// Relative tolerance for samples off the 50 us scope grid. Those are
    /// taken at LEDC events, on the edge itself, where the value depends on
    /// where the solver's step lands.
    pub off_grid_rel: f64,
}

impl Tolerance {
    /// E4 parity (Stefan, 25 November 2026): exact digital outputs, 1 ns edge
    /// times, `max(1 nA, 1 ppm)` on analog samples. For runs on the same
    /// machine.
    pub(crate) const PARITY: Self = Self {
        edge_ps: 1_000,
        abs_ma: 1e-6,
        rel: 1e-6,
        off_grid_rel: 1e-6,
    };
    /// Against the committed goldens, which were recorded on Linux x86-64 (the
    /// CI platform). Other platforms' libm moves samples taken on LEDC edges
    /// by up to about 0.35 % (seen on Windows MSVC in the fade demo); every
    /// other value stays within [`Self::PARITY`].
    pub(crate) const GOLDEN: Self = Self {
        off_grid_rel: 1e-2,
        ..Self::PARITY
    };
}

/// True when `t` (s) is on the 50 us scope grid.
fn on_grid(t: f64) -> bool {
    let n = (t / crate::trace::SAMPLE_STEP).round();
    (n * crate::trace::SAMPLE_STEP - t).abs() < 1e-12
}

/// Golden text for `trace`.
pub(crate) fn serialize(name: &str, trace: &DemoTrace) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "# ohmforge parity trace v1: {name}");
    let _ = writeln!(out, "uart {:?}", trace.uart);
    let _ = writeln!(out, "edges {}", trace.edges.len());
    for (t, pin, code) in &trace.edges {
        let _ = writeln!(out, "e {t} {pin} {code}");
    }
    let _ = writeln!(out, "samples {}", trace.samples.len());
    for (t, led, psu) in &trace.samples {
        let _ = writeln!(out, "s {t:.12e} {led:.12e} {psu:.12e}");
    }
    out
}

fn unquote(s: &str) -> Result<String, String> {
    let inner = s
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .ok_or_else(|| format!("bad uart string {s}"))?;
    let mut out = String::new();
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('t') => out.push('\t'),
            Some('0') => out.push('\0'),
            Some('\\') => out.push('\\'),
            Some('"') => out.push('"'),
            Some('\'') => out.push('\''),
            Some('u') => {
                let hex: String = chars.by_ref().skip(1).take_while(|&c| c != '}').collect();
                let v = u32::from_str_radix(&hex, 16).map_err(|e| e.to_string())?;
                out.push(char::from_u32(v).ok_or("bad escape")?);
            }
            other => return Err(format!("unknown escape {other:?}")),
        }
    }
    Ok(out)
}

/// Parse golden text back into a trace.
pub(crate) fn parse(text: &str) -> Result<DemoTrace, String> {
    let mut t = DemoTrace::default();
    for (n, line) in text.lines().enumerate() {
        let err = |m: &str| format!("line {}: {m}", n + 1);
        if line.starts_with('#') || line.is_empty() {
            continue;
        }
        let (tag, rest) = line.split_once(' ').ok_or_else(|| err("no tag"))?;
        let f: Vec<&str> = rest.split(' ').collect();
        match tag {
            "uart" => t.uart = unquote(rest).map_err(|e| err(&e))?,
            "edges" | "samples" => {}
            "e" if f.len() == 3 => t.edges.push((
                f[0].parse().map_err(|_| err("edge time"))?,
                f[1].parse().map_err(|_| err("edge pin"))?,
                f[2].to_owned(),
            )),
            "s" if f.len() == 3 => {
                let p = |s: &str| s.parse::<f64>().map_err(|_| err("sample value"));
                t.samples.push((p(f[0])?, p(f[1])?, p(f[2])?));
            }
            _ => return Err(err("unknown line")),
        }
    }
    Ok(t)
}

fn close(a: f64, b: f64, rel: f64, tol: Tolerance) -> bool {
    (a - b).abs() <= tol.abs_ma.max(rel * a.abs().max(b.abs()))
}

/// Compare `got` with the golden text. Returns the first difference.
pub(crate) fn compare(golden: &str, got: &DemoTrace, tol: Tolerance) -> Result<(), String> {
    compare_traces(&parse(golden)?, got, tol)
}

/// Compare two traces. Returns the first difference.
pub(crate) fn compare_traces(want: &DemoTrace, got: &DemoTrace, tol: Tolerance) -> Result<(), String> {
    if want.uart != got.uart {
        return Err(format!("UART differs: want {:?}, got {:?}", want.uart, got.uart));
    }
    if want.edges.len() != got.edges.len() {
        return Err(format!(
            "edge count differs: want {}, got {}",
            want.edges.len(),
            got.edges.len()
        ));
    }
    for (i, (w, g)) in want.edges.iter().zip(&got.edges).enumerate() {
        if w.1 != g.1 || w.2 != g.2 {
            return Err(format!("edge {i}: want {w:?}, got {g:?}"));
        }
        if w.0.abs_diff(g.0) > tol.edge_ps {
            return Err(format!("edge {i} time: want {} ps, got {} ps", w.0, g.0));
        }
    }
    if want.samples.len() != got.samples.len() {
        return Err(format!(
            "sample count differs: want {}, got {}",
            want.samples.len(),
            got.samples.len()
        ));
    }
    for (i, (w, g)) in want.samples.iter().zip(&got.samples).enumerate() {
        if (w.0 - g.0).abs() > 1e-12 {
            return Err(format!("sample {i} time: want {}, got {}", w.0, g.0));
        }
        let rel = if on_grid(w.0) { tol.rel } else { tol.off_grid_rel };
        if !close(w.1, g.1, rel, tol) {
            return Err(format!(
                "sample {i} at {:.6e} s, LED mA: want {}, got {}",
                w.0, w.1, g.1
            ));
        }
        if !close(w.2, g.2, rel, tol) {
            return Err(format!(
                "sample {i} at {:.6e} s, PSU mA: want {}, got {}",
                w.0, w.2, g.2
            ));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> DemoTrace {
        DemoTrace {
            edges: vec![(5, 8, "H2".into()), (10, 8, "L2".into())],
            uart: "hi \"x\"\n".into(),
            samples: vec![(0.0, 1.0, 2.0), (5e-5, 3.25, -1e-9)],
        }
    }

    #[test]
    fn round_trips_through_text() {
        let t = sample();
        let text = serialize("x", &t);
        let back = parse(&text).unwrap();
        assert_eq!(back.uart, t.uart);
        assert_eq!(back.edges, t.edges);
        assert_eq!(back.samples.len(), 2);
        assert!(compare(&text, &t, Tolerance::PARITY).is_ok());
        assert_eq!(serialize("x", &back), text);
    }

    #[test]
    fn parity_tolerance_catches_real_differences() {
        let t = sample();
        let text = serialize("x", &t);
        let mut g = t.clone();
        g.samples[1].1 += 1e-7; // 0.1 nA: inside max(1 nA, 1 ppm)
        assert!(compare(&text, &g, Tolerance::PARITY).is_ok());
        g.samples[1].1 += 1e-5; // 10 nA: outside
        assert!(compare(&text, &g, Tolerance::PARITY).is_err());
        let mut g = t.clone();
        g.edges[0].0 += 999;
        assert!(compare(&text, &g, Tolerance::PARITY).is_ok());
        g.edges[0].0 += 2;
        assert!(compare(&text, &g, Tolerance::PARITY).is_err());
        let mut g = t.clone();
        g.uart.push('!');
        assert!(compare(&text, &g, Tolerance::PARITY).is_err());
        // Off the grid (an LEDC event sample) the golden tolerance is 1 %.
        let mut t = t;
        t.samples.push((2.6275e-3, 3.957283560618, 3.96));
        let text = serialize("x", &t);
        let mut g = t.clone();
        g.samples[2].1 = 3.957349680679;
        assert!(compare(&text, &g, Tolerance::PARITY).is_err());
        assert!(compare(&text, &g, Tolerance::GOLDEN).is_ok());
    }
}
