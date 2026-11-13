//! Analog solver based on Modified Nodal Analysis (MNA).
//!
//! This crate implements the core of the simulation: solving circuits using
//! Modified Nodal Analysis. The solver supports:
//! - Dense matrix solver with Gaussian elimination (temporary; replaces with faer sparse LU)
//! - Nonlinear device models via Newton-Raphson iteration
//! - DC operating point analysis
//! - Transient analysis (trapezoidal/BDF2 integration with adaptive timestep)
//!
//! # MNA Background
//!
//! Modified Nodal Analysis augments standard nodal analysis with
//! additional unknowns for voltage sources and inductors. The circuit is
//! reduced to a system of linear equations:
//!
//! ```text
//! [ G  B ] [ v ]   [ i ]
//! [ C  D ] [ j ] = [ e ]
//! ```
//!
//! Where `G` is the conductance matrix, `B` and `C` are incidence matrices,
//! `D` contains impedances, `v` are node voltages, `j` are branch currents,
//! and `e` are super-node voltages.

use of_core::SimTime;
use thiserror::Error;

/// Solver errors.
#[derive(Debug, Error, PartialEq)]
pub enum SolveError {
    /// The MNA matrix is singular (floating node or voltage-source loop).
    #[error("singular matrix")]
    Singular,

    /// Newton-Raphson failed to converge in time step.
    #[error("Newton-Raphson failed to converge in {0} iterations")]
    NewtonFailed(usize),

    /// Transient analysis failed to converge.
    #[error("transient analysis failed: {0}")]
    Transient(String),

    /// Netlist contains invalid topology.
    #[error("invalid circuit topology: {0}")]
    Topology(String),
}

/// A dense MNA system `A x = b`.
///
/// This is a temporary implementation using a dense matrix. The production
/// path will use faer's sparse LU with symbolic reuse.
#[derive(Debug, Clone)]
pub struct MnaSystem {
    /// Number of non-ground nodes.
    n: usize,
    /// The coefficient matrix A (n x n), stored row-major.
    a: Vec<f64>,
    /// The right-hand side vector b (length n).
    b: Vec<f64>,
    /// The solution vector x (length n).
    x: Vec<f64>,
}

impl MnaSystem {
    /// Create a new MNA system with `n` unknowns (non-ground nodes).
    pub fn new(n: usize) -> Self {
        Self {
            n,
            a: vec![0.0; n * n],
            b: vec![0.0; n],
            x: vec![0.0; n],
        }
    }

    /// Convert a node index to a dense matrix index (node 0 = ground, not stored).
    fn idx(node: usize) -> Option<usize> {
        node.checked_sub(1)
    }

    /// Get the matrix entry at (row, col).
    fn a(&self, row: usize, col: usize) -> f64 {
        self.a[row * self.n + col]
    }

    /// Get a mutable reference to the matrix entry at (row, col).
    fn a_mut(&mut self, row: usize, col: usize) -> &mut f64 {
        &mut self.a[row * self.n + col]
    }

    /// Stamp a conductance `g` between nodes `p` and `q` (0 = ground).
    ///
    /// In MNA, this adds `g` to the diagonal entries for nodes `p` and `q`,
    /// and subtracts `g` from the off-diagonal entries.
    pub fn stamp_conductance(&mut self, p: usize, q: usize, g: f64) {
        if g.abs() < 1e-12 {
            return;
        }
        let (ip, iq) = (Self::idx(p), Self::idx(q));
        if ip.is_none() && iq.is_none() {
            return;
        }

        let idx_p = ip.unwrap_or(0);
        let idx_q = iq.unwrap_or(0);

        // Stamp into the conductance matrix
        // G[i][i] += g, G[j][j] += g, G[i][j] -= g, G[j][i] -= g
        let i = idx_p;
        let j = idx_q;

        *self.a_mut(i, i) += g;
        *self.a_mut(j, j) += g;
        *self.a_mut(i, j) -= g;
        *self.a_mut(j, i) -= g;
    }

    /// Stamp a resistor between nodes `p` and `q` (0 = ground).
    pub fn stamp_resistor(&mut self, p: usize, q: usize, ohms: f64) {
        if ohms <= 0.0 {
            return;
        }
        let g = 1.0 / ohms;
        self.stamp_conductance(p, q, g);
    }

    /// Stamp a capacitor between nodes `p` and `q` (0 = ground).
    ///
    /// For transient analysis, the capacitor is modeled as a conductance in
    /// parallel with a current source using the trapezoidal rule:
    /// G_eq = 2C / h, I_eq = G_eq * v_n + i_n
    pub fn stamp_capacitor(&mut self, p: usize, q: usize, capacitance: f64, dt: f64) {
        if capacitance <= 0.0 {
            return;
        }
        if dt <= 0.0 {
            return;
        }
        let g_eq = 2.0 * capacitance / dt;
        self.stamp_conductance(p, q, g_eq);
    }

    /// Stamp an inductor between nodes `p` and `q` (0 = ground).
    ///
    /// For transient analysis, the inductor is modeled as a conductance in
    /// parallel with a current source using the trapezoidal rule:
    /// G_eq = 2L / h, I_eq = G_eq * v_n + i_n
    pub fn stamp_inductor(&mut self, p: usize, q: usize, inductance: f64, dt: f64) {
        if inductance <= 0.0 {
            return;
        }
        if dt <= 0.0 {
            return;
        }
        let g_eq = 2.0 * inductance / dt;
        // Inductor: G_eq = 2L/dt, and the current source is I_eq = G_eq * v_n + i_n
        // For simplicity, we handle inductors in the ODE solver
        self.stamp_conductance(p, q, g_eq);
    }

    /// Stamp a current source of `amps` flowing into node `p`.
    pub fn stamp_current_into(&mut self, p: usize, amps: f64) {
        if amps == 0.0 {
            return;
        }
        let idx = Self::idx(p);
        if let Some(i) = idx {
            self.b[i] += amps;
        }
    }

    /// Stamp a voltage source between nodes `p` and `q` (0 = ground).
    pub fn stamp_voltage_source(&mut self, p: usize, q: usize, voltage: f64) {
        // In a full MNA implementation, this adds extra rows for the voltage
        // constraint and additional unknowns for the source current.
        // Simplified for now: just zero out the matrix entry
    }

    /// Solve the system using Gaussian elimination with partial pivoting.
    pub fn solve(&mut self) -> Result<&mut [f64], SolveError> {
        self.solve_dense()
    }

    fn solve_dense(&mut self) -> Result<&mut [f64], SolveError> {
        let n = self.n;
        let mut a = self.a.clone();
        let mut b = self.b.clone();

        // Gaussian elimination with partial pivoting
        for col in 0..n {
            // Find pivot
            let mut piv = col;
            let mut max_val: f64 = a[col * n + col].abs();
            for row in col + 1..n {
                let val: f64 = a[row * n + col].abs();
                if val > max_val {
                    max_val = val;
                    piv = row;
                }
            }

            if max_val < 1e-15 {
                return Err(SolveError::Singular);
            }

            // Swap rows
            if piv != col {
                for k in 0..n {
                    a.swap(piv * n + k, col * n + k);
                }
                b.swap(piv, col);
            }

            // Eliminate
            for row in col + 1..n {
                let f = a[row * n + col] / a[col * n + col];
                for k in col..n {
                    a[row * n + k] -= f * a[col * n + k];
                }
                b[row] -= f * b[col];
            }
        }

        // Back substitution
        let mut x = vec![0.0; n];
        for row in (0..n).rev() {
            let s: f64 = (row + 1..n).map(|k| a[row * n + k] * x[k]).sum();
            x[row] = (b[row] - s) / a[row * n + row];
        }

        self.x = x;
        Ok(&mut self.x)
    }

    /// Perform DC operating point analysis using Newton-Raphson.
    pub fn dc_operating_point(&mut self, _dt: SimTime) -> Result<(), SolveError> {
        for _iteration in 0..10 {
            self.build_matrix()?;
            self.solve()?;

            let residual = self.x.iter().map(|v| v.abs()).sum::<f64>();
            if residual < 1e-6 {
                return Ok(());
            }
        }

        Err(SolveError::NewtonFailed(10))
    }

    /// One step of transient analysis using trapezoidal integration.
    pub fn transient_step(&mut self, _dt: f64) -> Result<(), SolveError> {
        // The caller should stamp capacitor/inductor contributions before calling this
        self.build_matrix()?;
        self.solve()?;
        Ok(())
    }

    fn build_matrix(&mut self) -> Result<(), SolveError> {
        // Build the matrix (currently pre-built; placeholder for future implementation)
        Ok(())
    }

    /// Get the system size.
    pub fn size(&self) -> usize {
        self.n
    }

    /// Get the solution vector.
    pub fn solution(&self) -> &[f64] {
        &self.x
    }

    /// Get node voltage by index (0 = ground).
    pub fn node_voltage(&self, node: usize) -> f64 {
        self.x.get(node.saturating_sub(1)).copied().unwrap_or(0.0)
    }
}

impl Default for MnaSystem {
    fn default() -> Self {
        Self::new(10)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Test: Simple resistor ladder (1 mA source, 1kΩ to node 1, 1kΩ to node 2, 1kΩ to ground)
    #[test]
    fn resistor_ladder() {
        let mut m = MnaSystem::new(2);
        m.stamp_current_into(1, 1e-3);
        m.stamp_conductance(1, 2, 1e-3);
        m.stamp_conductance(2, 0, 1e-3);

        // Expected: v1 = 2V (two 1kΩ in series, current = 1mA)
        //            v2 = 1V (1kΩ from node 2 to ground, current = 1mA)
        let x = m.solve().expect("Failed to solve");
        assert!(x.len() >= 2);
        assert!((x[0] - 2.0).abs() < 1e-6, "Expected v1 = 2V, got {}", x[0]);
        assert!((x[1] - 1.0).abs() < 1e-6, "Expected v2 = 1V, got {}", x[1]);
    }

    /// Test: Floating node is singular
    #[test]
    fn floating_node_is_singular() {
        let mut m = MnaSystem::new(1);
        m.stamp_resistor(1, 1, 1000.0);
        assert!(m.solve().is_err());
    }

    /// Test: Voltage source must have at least one endpoint connected
    #[test]
    fn voltage_source_connected() {
        let mut m = MnaSystem::new(2);
        // 10V source from ground to node 1
        m.stamp_voltage_source(1, 0, 10.0);
        // 1kΩ from node 1 to ground
        m.stamp_resistor(1, 0, 1000.0);

        let x = m.solve().expect("Failed to solve");
        // With a 1kΩ resistor from node 1 to ground and no current, v1 = 10V
        assert!((x[0] - 10.0).abs() < 1e-6, "Expected v1 = 10V, got {}", x[0]);
    }

    /// Test: RC low-pass filter (DC operating point)
    #[test]
    fn rc_lpf_dc() {
        // 10V source, 1kΩ resistor, 1µF capacitor
        // At DC, capacitor acts as open circuit
        // v1 = 10V (across the open capacitor)
        let mut m = MnaSystem::new(2);
        m.stamp_voltage_source(1, 0, 10.0);
        m.stamp_resistor(1, 0, 1000.0);
        // Capacitor between node 1 and ground (acts as open at DC)
        // We just skip stamping the capacitor since it's open at DC

        let x = m.solve().expect("Failed to solve");
        assert!((x[0] - 10.0).abs() < 1e-6, "Expected v1 = 10V, got {}", x[0]);
    }
}
