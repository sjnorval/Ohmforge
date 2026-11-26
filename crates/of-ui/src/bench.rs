//! Real-world profiles on the desktop bench (engine E5f, RFC 0004).
//!
//! The meters, the scope and the PSU readout show what the chosen instrument
//! would: the Fluke 87V's autoranged value with its ± band and `OL`, the
//! DS1054Z's 8-bit trace (V/div ÷ 25 per code, clipped) and the DP832's
//! rounded readback with its bands. Ideal (the default) shows the solver's
//! values unchanged. Display only: the desktop does not re-elaborate the
//! circuit with a meter's loading mid-run (circuit files and `ohmforge run` do).

use of_instruments::Reading;
use of_instruments::profile::{AmpInput, DmmProfile, PsuProfile, Readback, ScopeProfile};

use crate::supply::SupplyStatus;

/// The profile behind each bench instrument (`None`: ideal).
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct BenchProfiles {
    /// Multimeters (amps on the mA jack).
    pub meter: Option<&'static DmmProfile>,
    /// The LED scope.
    pub scope: Option<&'static ScopeProfile>,
    /// The PSU readout (CH1).
    pub psu: Option<&'static PsuProfile>,
}

impl BenchProfiles {
    /// A meter's text: ideal `1.650 V` / `22.98 mA`, or the profile's
    /// `1.649 ± 0.002 V`, `OL (60.00 V)`.
    pub fn meter_text(&self, reading: &Reading) -> String {
        self.meter
            .and_then(|p| reading.on(p, AmpInput::Milli))
            .map_or_else(|| reading.display(), |d| d.to_string())
    }

    /// Quantise a trace in place at `per_div` (in the trace's unit) as the
    /// scope's ADC would, with the trace's zero at the reference code.
    pub fn scope_trace(&self, trace: &mut [(f64, f64)], per_div: f64) {
        if let Some(s) = self.scope {
            for (_, y) in trace {
                *y = s.quantise(*y, per_div, 0.0);
            }
        }
    }

    /// The scope's step and ± band at `per_div`, e.g. `Rigol DS1054Z: 0.08 mA/code, ± 0.40 mA`.
    pub fn scope_note(&self, per_div: f64, unit: &str) -> Option<String> {
        self.scope.map(|s| {
            format!(
                "{}: {:.2} {unit}/code, \u{b1} {:.2} {unit}",
                s.name,
                s.step(per_div),
                s.relative_band(per_div)
            )
        })
    }

    /// Round the PSU status to CH1's readback; the bands, when a profile is set.
    pub fn supply(&self, status: &mut SupplyStatus) -> Option<Readback> {
        let ch = self.psu?.channels.first()?;
        let rb = ch.readback(status.volts, status.display_amps());
        status.volts = rb.volts;
        status.amps = ch.readback(0.0, status.amps).amps;
        if status.amps_avg.is_some() {
            status.amps_avg = Some(rb.amps);
        }
        Some(rb)
    }
}

#[cfg(test)]
mod tests {
    use of_instruments::profile::{multimeters, scopes, supplies};

    use super::*;

    fn real() -> BenchProfiles {
        BenchProfiles {
            meter: multimeters().find(|p| p.id == "fluke.87v"),
            scope: scopes().find(|p| p.id == "rigol.ds1054z"),
            psu: supplies().find(|p| p.id == "rigol.dp832"),
        }
    }

    #[test]
    fn ideal_is_the_default_and_unchanged() {
        let b = BenchProfiles::default();
        assert_eq!(b.meter_text(&Reading::Volts(1.6492)), "1.649 V");
        let mut t = vec![(0.0, 1.234_567)];
        b.scope_trace(&mut t, 2.0);
        assert_eq!(t[0].1, 1.234_567);
        assert!(b.scope_note(2.0, "mA").is_none());
        let mut s = SupplyStatus {
            volts: 3.2977,
            ..SupplyStatus::default()
        };
        assert!(b.supply(&mut s).is_none());
        assert_eq!(s.volts, 3.2977);
    }

    #[test]
    fn real_profiles_band_range_and_quantise() {
        let b = real();
        assert_eq!(b.meter_text(&Reading::Volts(1.6492)), "1.649 \u{b1} 0.002 V");
        let amps = Reading::Amps {
            amps: 22.98e-3,
            part: "R1".into(),
        };
        assert_eq!(b.meter_text(&amps), "22.98 \u{b1} 0.09 mA");
        assert_eq!(b.meter_text(&Reading::Volts(2000.0)), "OL (1000 V)");
        assert_eq!(b.meter_text(&Reading::Missing), "\u{2014}");
        // 2 mA/div: 0.08 mA per code; 1.23 mA -> 15 codes = 1.20 mA.
        let mut t = vec![(0.0, 1.23), (1.0, 100.0)];
        b.scope_trace(&mut t, 2.0);
        assert!((t[0].1 - 1.20).abs() < 1e-12, "{t:?}");
        assert!((t[1].1 - 128.0 * 0.08).abs() < 1e-12, "{t:?}");
        // 3 % of 8 div + 0.1 div at 2 mA/div = 0.68 mA.
        assert_eq!(
            b.scope_note(2.0, "mA").unwrap(),
            "Rigol DS1054Z: 0.08 mA/code, \u{b1} 0.68 mA"
        );
        let mut s = SupplyStatus {
            volts: 3.2977,
            amps: 0.02298,
            amps_avg: Some(0.0114),
            ..SupplyStatus::default()
        };
        let rb = b.supply(&mut s).unwrap();
        assert!(
            (s.volts - 3.30).abs() < 1e-12 && (s.amps - 0.023).abs() < 1e-12,
            "{s:?}"
        );
        assert!((s.amps_avg.unwrap() - 0.011).abs() < 1e-12, "{s:?}");
        assert!((rb.volts_band - (0.0005 * 3.30 + 0.010)).abs() < 1e-12);
    }
}
