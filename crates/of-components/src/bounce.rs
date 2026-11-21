//! Contact bounce for the `generic.pushbutton` part.
//!
//! The part itself is still an ideal switch (`ron` / `roff`). Bounce is a
//! scripted sequence of close/open events, shared by the firmware
//! co-simulation (`tests/esp_hal_button.rs`) and the live bench. Seed 0
//! plays the canonical train used by those tests; any other seed applies a
//! deterministic jitter of the same edges.

/// Close-open chatter on a press, microseconds after the press instant.
/// `true` means the contact is closed.
pub const PRESS_BOUNCE_US: &[(f64, bool)] = &[(0.0, true), (30.0, false), (60.0, true), (100.0, false), (150.0, true)];

/// Close-open chatter on a release, microseconds after the release instant.
pub const RELEASE_BOUNCE_US: &[(f64, bool)] = &[(0.0, false), (20.0, true), (50.0, false)];

/// Seeded, toggleable bounce for a press or release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BounceSettings {
    /// When false, a press or release is a single clean edge at t = 0.
    pub enabled: bool,
    /// Seed 0 plays [`PRESS_BOUNCE_US`] / [`RELEASE_BOUNCE_US`] exactly; any
    /// other seed applies a deterministic jitter (same number of edges and
    /// polarity, intervals stay in the same tens-of-microseconds range).
    pub seed: u64,
}

impl Default for BounceSettings {
    fn default() -> Self {
        Self { enabled: true, seed: 0 }
    }
}

/// Bounce edges for a press (`closed` = true) or release, as `(seconds, closed)`.
pub fn bounce_script(closed: bool, settings: BounceSettings) -> Vec<(f64, bool)> {
    if !settings.enabled {
        return vec![(0.0, closed)];
    }
    let base = if closed { PRESS_BOUNCE_US } else { RELEASE_BOUNCE_US };
    if settings.seed == 0 {
        return base.iter().map(|&(us, c)| (us * 1e-6, c)).collect();
    }
    jitter(base, settings.seed)
}

fn jitter(base: &[(f64, bool)], seed: u64) -> Vec<(f64, bool)> {
    let mut rng = seed ^ 0x9E37_79B9_7F4A_7C15;
    let mut out = Vec::with_capacity(base.len());
    let mut t_us = 0.0;
    let mut prev_us = 0.0;
    for (i, &(us, c)) in base.iter().enumerate() {
        if i == 0 {
            out.push((0.0, c));
            prev_us = us;
            continue;
        }
        rng = rng.wrapping_mul(0x5851_F42D_4C95_7F2D).wrapping_add(1);
        let u = (rng >> 33) as f64 / ((1u64 << 31) as f64);
        t_us += (us - prev_us).max(0.0) * (0.7 + 0.6 * u);
        prev_us = us;
        out.push((t_us * 1e-6, c));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_zero_is_the_canonical_train() {
        let press = bounce_script(true, BounceSettings { enabled: true, seed: 0 });
        assert_eq!(
            press,
            PRESS_BOUNCE_US
                .iter()
                .map(|&(us, c)| (us * 1e-6, c))
                .collect::<Vec<_>>()
        );
        let release = bounce_script(false, BounceSettings { enabled: true, seed: 0 });
        assert_eq!(
            release,
            RELEASE_BOUNCE_US
                .iter()
                .map(|&(us, c)| (us * 1e-6, c))
                .collect::<Vec<_>>()
        );
    }

    #[test]
    fn disabled_is_a_single_clean_edge() {
        let off = BounceSettings {
            enabled: false,
            seed: 99,
        };
        assert_eq!(bounce_script(true, off), vec![(0.0, true)]);
        assert_eq!(bounce_script(false, off), vec![(0.0, false)]);
    }

    #[test]
    fn other_seeds_jitter_deterministically() {
        let a = BounceSettings { enabled: true, seed: 1 };
        let b = BounceSettings { enabled: true, seed: 2 };
        let p1 = bounce_script(true, a);
        let p1b = bounce_script(true, a);
        let p2 = bounce_script(true, b);
        assert_eq!(p1, p1b);
        assert_ne!(p1, p2);
        assert_eq!(p1.len(), PRESS_BOUNCE_US.len());
        assert_eq!(
            p1.iter().map(|e| e.1).collect::<Vec<_>>(),
            PRESS_BOUNCE_US.iter().map(|e| e.1).collect::<Vec<_>>()
        );
        for w in p1.windows(2) {
            assert!(w[1].0 > w[0].0);
            // Jittered intervals stay in the same tens-of-microseconds range.
            assert!((w[1].0 - w[0].0) > 5e-6 && (w[1].0 - w[0].0) < 200e-6);
        }
    }
}
