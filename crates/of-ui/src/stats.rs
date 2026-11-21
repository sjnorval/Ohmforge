//! Lightweight counters for live-playback profiling (not on the hot path).

use of_analog::AnalogEngine;
use of_analog::results::TranStats;
use of_core::{EngineId, Kernel};
use of_soc_esp32c3::Esp32c3Engine;

/// Analog solver and hart counters at one instant.
#[derive(Clone, Copy, Debug)]
pub(crate) struct PerfSnapshot {
    pub time: f64,
    pub accepted: usize,
    pub rejected_lte: usize,
    pub rejected_newton: usize,
    pub newton_iterations: usize,
    pub factorisations: u64,
    pub breakpoints: usize,
    pub minstret: u64,
    pub mcycle: u64,
    pub samples: usize,
}

impl PerfSnapshot {
    pub(crate) fn from_kernel(kernel: &mut Kernel, cpu: EngineId, analog: EngineId, time: f64, samples: usize) -> Self {
        let st: TranStats = kernel
            .engine_as::<AnalogEngine>(analog)
            .expect("analog solver is not registered")
            .stats();
        let hart = &kernel
            .engine_as::<Esp32c3Engine>(cpu)
            .expect("registered ESP32-C3")
            .soc
            .hart;
        let csr = hart.csrs();
        Self {
            time,
            accepted: st.accepted,
            rejected_lte: st.rejected_lte,
            rejected_newton: st.rejected_newton,
            newton_iterations: st.newton_iterations,
            factorisations: st.factorisations,
            breakpoints: st.breakpoints,
            minstret: csr.minstret,
            mcycle: csr.mcycle,
            samples,
        }
    }

    pub(crate) fn delta(self, later: Self) -> Self {
        Self {
            time: later.time - self.time,
            accepted: later.accepted.saturating_sub(self.accepted),
            rejected_lte: later.rejected_lte.saturating_sub(self.rejected_lte),
            rejected_newton: later.rejected_newton.saturating_sub(self.rejected_newton),
            newton_iterations: later.newton_iterations.saturating_sub(self.newton_iterations),
            factorisations: later.factorisations.saturating_sub(self.factorisations),
            breakpoints: later.breakpoints.saturating_sub(self.breakpoints),
            minstret: later.minstret.saturating_sub(self.minstret),
            mcycle: later.mcycle.saturating_sub(self.mcycle),
            samples: later.samples.saturating_sub(self.samples),
        }
    }
}
