//! The bench PSU's face on the measurement service (engine E5d, RFC 0004).
//!
//! [`PsuReadout`] watches a `bench.psu`'s `r(PSU.regulated_amps)` in a
//! service that the run loop records *after* each commit (the component's
//! readings describe the committed step), and builds the face
//! ([`SupplyStatus`]) from `r(PSU.volts)`, `r(PSU.regulated_amps)` and
//! `r(PSU.mode)`. The face's current is averaged over whole PWM periods of
//! the LED current, exactly as the desktop did before E5d (in mA, so the
//! numbers are unchanged).

use of_components::psu::{SupplyMode, SupplySettings, SupplyStatus};

use crate::service::{ChannelId, MeasurementService, Probeable};
use crate::signal::Signal;
use crate::stats::average_periods;

/// A bench PSU's face, read through the measurement service.
#[derive(Debug, Clone)]
pub struct PsuReadout {
    refdes: String,
    amps: ChannelId,
}

impl PsuReadout {
    /// Watch `refdes`'s regulated current in `settled` (recorded after commits).
    pub fn watch(settled: &mut MeasurementService, refdes: &str) -> Self {
        let amps = settled.watch_default(Self::signal(refdes, "regulated_amps"));
        Self {
            refdes: refdes.to_owned(),
            amps,
        }
    }

    fn signal(refdes: &str, name: &str) -> Signal {
        Signal::Reading {
            refdes: refdes.to_owned(),
            name: name.to_owned(),
        }
    }

    fn reading<P: Probeable + ?Sized>(&self, src: &mut P, name: &str) -> Option<f64> {
        src.sample(&Self::signal(&self.refdes, name)).ok()
    }

    /// The face now (no average), from the readings after the last commit.
    /// `settings` are the user's set points (for the rating warning). A
    /// missing reading reads 0 (mode: off).
    pub fn status<P: Probeable + ?Sized>(&self, src: &mut P, settings: SupplySettings) -> SupplyStatus {
        let volts = self.reading(src, "volts").unwrap_or(0.0);
        let amps = self.reading(src, "regulated_amps").unwrap_or(0.0);
        let mode = match self.reading(src, "mode").map(|v| v as i64) {
            Some(1) => SupplyMode::Cv,
            Some(2) => SupplyMode::Cc,
            _ => SupplyMode::Off,
        };
        // The load's rating, as the component reports it (from the parts on
        // its + rail); no rating, no warning.
        let rated = match (self.reading(src, "rated_min"), self.reading(src, "rated_max")) {
            (Some(lo), Some(hi)) => Some((lo, hi)),
            _ => settings.rated,
        };
        let outside_rated = rated.is_some_and(|(lo, hi)| {
            let rated = lo..=hi;
            if settings.on {
                !rated.contains(&settings.volts) || !rated.contains(&volts)
            } else {
                !rated.contains(&volts) && volts > 0.05
            }
        });
        SupplyStatus {
            volts,
            amps,
            amps_avg: None,
            mode,
            outside_rated,
        }
    }

    /// The recorded current in mA (the scope's and goldens' unit).
    pub fn samples_ma(&self, settled: &MeasurementService) -> Vec<(f64, f64)> {
        settled
            .channel(self.amps)
            .map(|c| c.ring.iter().map(|(t, a)| (t, a * 1e3)).collect())
            .unwrap_or_default()
    }

    /// Period-averaged current (A): the recorded current over `periods`
    /// whole periods of `reference_ma` (rising through `high_ma`), or over
    /// `window` seconds. Averaged in mA as before E5d.
    pub fn average_amps(
        &self,
        settled: &MeasurementService,
        reference_ma: &[(f64, f64)],
        now: f64,
        high_ma: f64,
        periods: usize,
        window: f64,
    ) -> f64 {
        average_periods(&self.samples_ma(settled), reference_ma, now, high_ma, periods, window) * 1e-3
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ProbeError, SamplePlan};

    struct Bench {
        t: f64,
        volts: f64,
        amps: f64,
        mode: f64,
    }

    impl Probeable for Bench {
        fn now_secs(&self) -> f64 {
            self.t
        }
        fn sample(&mut self, signal: &Signal) -> Result<f64, ProbeError> {
            match signal {
                Signal::Reading { refdes, name } if refdes == "PSU" => match name.as_str() {
                    "volts" => Ok(self.volts),
                    "regulated_amps" => Ok(self.amps),
                    "mode" => Ok(self.mode),
                    // An ESP32-C3 on the rail (vdd 3.0 to 3.6 V).
                    "rated_min" => Ok(3.0),
                    "rated_max" => Ok(3.6),
                    _ => Err(ProbeError::Unresolved(signal.clone())),
                },
                _ => Err(ProbeError::Unresolved(signal.clone())),
            }
        }
    }

    #[test]
    fn the_face_comes_from_the_components_readings() {
        let mut svc = MeasurementService::new(SamplePlan::default());
        let psu = PsuReadout::watch(&mut svc, "PSU");
        let mut b = Bench {
            t: 0.0,
            volts: 3.298,
            amps: 0.023,
            mode: 1.0,
        };
        let st = psu.status(&mut b, SupplySettings::default());
        assert_eq!(
            (st.volts, st.amps, st.mode, st.outside_rated),
            (3.298, 0.023, SupplyMode::Cv, false)
        );
        assert_eq!(st.amps_avg, None);
        b.mode = 2.0;
        b.volts = 2.5;
        let st = psu.status(&mut b, SupplySettings::default());
        assert_eq!((st.mode, st.outside_rated), (SupplyMode::Cc, true));
        b.mode = 0.0;
        b.volts = 0.0;
        let off = SupplySettings {
            on: false,
            ..SupplySettings::default()
        };
        let st = psu.status(&mut b, off);
        assert_eq!((st.mode, st.outside_rated), (SupplyMode::Off, false));
    }

    #[test]
    fn the_average_is_the_desktops_ma_average() {
        let mut svc = MeasurementService::new(SamplePlan::default());
        let psu = PsuReadout::watch(&mut svc, "PSU");
        let mut b = Bench {
            t: 0.0,
            volts: 3.3,
            amps: 0.0,
            mode: 1.0,
        };
        let mut led = Vec::new();
        let mut psu_ma = Vec::new();
        for k in 0..400 {
            b.t = k as f64 * 50e-6;
            let on = (k / 10) % 2 == 0;
            b.amps = if on { 0.0273 } else { 0.0231 } + k as f64 * 1e-7;
            svc.record(&mut b).unwrap();
            led.push((b.t, if on { 4.0 } else { 0.0 }));
            psu_ma.push((b.t, b.amps * 1e3));
        }
        let now = b.t;
        let want = average_periods(&psu_ma, &led, now, 0.5, 2, 2e-3) * 1e-3;
        assert_eq!(psu.average_amps(&svc, &led, now, 0.5, 2, 2e-3), want);
        assert_eq!(psu.samples_ma(&svc), psu_ma);
    }
}
