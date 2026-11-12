//! The stable API every component implements, built-in or plugin.
//!
//! See docs/DESIGN_AND_PLAN.md, section 7.

use of_core::SimTime;

/// Result of committing a step: whether the part is still within ratings.
#[derive(Debug, Clone, PartialEq)]
pub enum Health {
    /// Within ratings.
    Ok,
    /// Near or above a rating (for example junction temperature).
    Warning(String),
    /// Absolute maximum exceeded; the real part would be damaged.
    Damaged(String),
}

/// Faults a user can inject into a part.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FaultKind {
    /// Open circuit between two pins.
    Open,
    /// Short circuit between two pins.
    Short,
    /// Pin stuck at a logic level.
    StuckAt,
    /// Parameter drifted out of tolerance.
    Drift,
}

/// Static information about a component instance.
#[derive(Debug, Clone, Default)]
pub struct ComponentInfo {
    /// Library part id, for example `bosch.bme280`.
    pub part_id: String,
    /// Pin names in order.
    pub pins: Vec<String>,
}

/// Behaviour of a simulated part.
pub trait Component: Send {
    /// Pins and parameters, resolved from the part's `part.toml`.
    fn describe(&self) -> &ComponentInfo;
    /// Accept a step: update internal state, dissipation and temperature.
    fn commit(&mut self, _t: SimTime) -> Health {
        Health::Ok
    }
    /// Faults this part supports.
    fn faults(&self) -> &[FaultKind] {
        &[]
    }
}
