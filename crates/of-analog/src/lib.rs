//! Analog solver based on Modified Nodal Analysis (MNA).
//!
//! This stub solves a small dense system with partial pivoting so the stamp
//! API can be exercised in tests. The production path will use faer's sparse
//! LU with symbolic reuse, Newton-Raphson for nonlinear devices and
//! trapezoidal/BDF2 integration with local-truncation-error step control.

use thiserror::Error;

/// Solver errors.
#[derive(Debug, Error, PartialEq)]
pub enum SolveError {
    /// The MNA matrix is singular (floating node or voltage-source loop).
    #[error("singular matrix at row {0}")]
    Singular(usize),
}

/// A dense MNA system `A x = b` (stub; to be replaced with sparse storage).
#[derive(Debug, Clone)]
pub struct MnaSystem {
    n: usize,
    a: Vec<f64>,
    b: Vec<f64>,
}

impl MnaSystem {
    /// System with `n` unknowns (non-ground nodes plus branch currents).
    pub fn new(n: usize) -> Self {
        Self {
            n,
            a: vec![0.0; n * n],
            b: vec![0.0; n],
        }
    }

    fn idx(node: usize) -> Option<usize> {
        node.checked_sub(1) // node 0 is ground and is not stored
    }

    /// Stamp a conductance `g` between nodes `p` and `q` (0 = ground).
    pub fn stamp_conductance(&mut self, p: usize, q: usize, g: f64) {
        let (ip, iq) = (Self::idx(p), Self::idx(q));
        if let Some(i) = ip {
            self.a[i * self.n + i] += g;
        }
        if let Some(j) = iq {
            self.a[j * self.n + j] += g;
        }
        if let (Some(i), Some(j)) = (ip, iq) {
            self.a[i * self.n + j] -= g;
            self.a[j * self.n + i] -= g;
        }
    }

    /// Stamp an independent current source of `amps` flowing into node `p`.
    pub fn stamp_current_into(&mut self, p: usize, amps: f64) {
        if let Some(i) = Self::idx(p) {
            self.b[i] += amps;
        }
    }

    /// Solve by Gaussian elimination with partial pivoting.
    pub fn solve(mut self) -> Result<Vec<f64>, SolveError> {
        let n = self.n;
        for col in 0..n {
            let piv = (col..n)
                .max_by(|&r1, &r2| self.a[r1 * n + col].abs().total_cmp(&self.a[r2 * n + col].abs()))
                .unwrap_or(col);
            if self.a[piv * n + col].abs() < 1e-15 {
                return Err(SolveError::Singular(col));
            }
            if piv != col {
                for k in 0..n {
                    self.a.swap(piv * n + k, col * n + k);
                }
                self.b.swap(piv, col);
            }
            for row in col + 1..n {
                let f = self.a[row * n + col] / self.a[col * n + col];
                for k in col..n {
                    self.a[row * n + k] -= f * self.a[col * n + k];
                }
                self.b[row] -= f * self.b[col];
            }
        }
        let mut x = vec![0.0; n];
        for row in (0..n).rev() {
            let s: f64 = (row + 1..n).map(|k| self.a[row * n + k] * x[k]).sum();
            x[row] = (self.b[row] - s) / self.a[row * n + row];
        }
        Ok(x)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Norton source 1 mA into node 1, 1 kΩ to node 2, 1 kΩ from node 2 to ground.
    #[test]
    fn resistor_ladder() {
        let mut m = MnaSystem::new(2);
        m.stamp_current_into(1, 1e-3);
        m.stamp_conductance(1, 2, 1e-3);
        m.stamp_conductance(2, 0, 1e-3);
        let v = m.solve().unwrap();
        assert!((v[0] - 2.0).abs() < 1e-9);
        assert!((v[1] - 1.0).abs() < 1e-9);
    }

    #[test]
    fn floating_node_is_singular() {
        let m = MnaSystem::new(1);
        assert_eq!(m.solve(), Err(SolveError::Singular(0)));
    }
}
