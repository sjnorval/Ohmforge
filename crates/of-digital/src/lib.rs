//! Event-driven digital kernel for nets with only digital endpoints.

/// IEEE-1164-style 4-state logic value (subset).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Logic {
    /// Driven low.
    Low,
    /// Driven high.
    High,
    /// High impedance (undriven).
    #[default]
    HighZ,
    /// Unknown or contention.
    Unknown,
}

impl Logic {
    /// Resolve two drivers on the same net.
    pub fn resolve(self, other: Self) -> Self {
        match (self, other) {
            (Logic::HighZ, x) | (x, Logic::HighZ) => x,
            (a, b) if a == b => a,
            _ => Logic::Unknown,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::Logic::*;

    #[test]
    fn contention_is_unknown() {
        assert_eq!(High.resolve(HighZ), High);
        assert_eq!(High.resolve(Low), Unknown);
    }
}
