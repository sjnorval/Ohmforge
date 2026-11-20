//! Random number generator (TRM chapter "Random Number Generator"):
//! `RNG_DATA_REG` at `0x6002_60B0`, in the APB_CTRL (SYSCON) block.
//!
//! On hardware each read returns 32 bits mixed from thermal and RF noise.
//! The simulation must stay deterministic, so this is a fixed-seed
//! xorshift32 generator that steps on every read: the values look random to
//! firmware (never stuck at 0, which the ESP-IDF bootloader waits on) and
//! repeat from run to run. They are not suitable for cryptography, and
//! neither is anything derived from them in a simulation.

/// Offset of `RNG_DATA_REG` from [`crate::map::APB_CTRL_BASE`].
pub const DATA: u32 = 0xb0;

const SEED: u32 = 0x6f68_6d66; // "ohmf"

/// The RNG data register.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rng {
    state: u32,
}

impl Default for Rng {
    fn default() -> Self {
        Self::new()
    }
}

impl Rng {
    /// Reset state (fixed seed).
    pub fn new() -> Self {
        Self { state: SEED }
    }

    /// Read `RNG_DATA_REG`: the next value.
    pub fn next_word(&mut self) -> u32 {
        let mut x = self.state;
        x ^= x << 13;
        x ^= x >> 17;
        x ^= x << 5;
        self.state = x;
        x
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn values_change_on_every_read_and_repeat_per_run() {
        let mut a = Rng::new();
        let v: Vec<u32> = (0..1000).map(|_| a.next_word()).collect();
        assert!(v.iter().all(|&w| w != 0));
        assert!(v.windows(2).all(|w| w[0] != w[1]));
        let mut b = Rng::new();
        assert!(v.iter().all(|&w| w == b.next_word()), "deterministic");
        // Both halves of the word vary (no stuck bits).
        let or = v.iter().fold(0, |acc, w| acc | w);
        let and = v.iter().fold(u32::MAX, |acc, w| acc & w);
        assert_eq!((or, and), (u32::MAX, 0));
    }
}
