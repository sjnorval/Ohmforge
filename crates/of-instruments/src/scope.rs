//! The oscilloscope's trace on the measurement service (engine E5d,
//! RFC 0004): a [`Scope`] shows one watched channel, raw or as the moving
//! average over whole PWM periods (the desktop's PWM demos). Time/div,
//! trigger and hold stay in the panel.

use crate::service::{ChannelId, MeasurementService};
use crate::stats::average_periods;

/// What the trace shows.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum ScopeTrace {
    /// The samples as recorded.
    Raw,
    /// At each sample, the average over `periods` whole periods of the
    /// trace itself (rising through `high`, in display units), or over
    /// `fallback` seconds when no period is seen.
    PeriodAverage {
        /// Rising-edge threshold (display units).
        high: f64,
        /// Whole periods averaged.
        periods: usize,
        /// Window (s) without a period.
        fallback: f64,
    },
}

/// A scope channel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Scope {
    /// The watched channel.
    pub channel: ChannelId,
    /// Display units per signal unit (1e3 shows amps in mA).
    pub scale: f64,
    /// Raw or averaged.
    pub mode: ScopeTrace,
}

impl Scope {
    /// The channel's samples in display units.
    pub fn samples(&self, svc: &MeasurementService) -> Vec<(f64, f64)> {
        svc.channel(self.channel)
            .map(|c| c.ring.iter().map(|(t, v)| (t, v * self.scale)).collect())
            .unwrap_or_default()
    }

    /// The trace to draw.
    pub fn trace(&self, svc: &MeasurementService) -> Vec<(f64, f64)> {
        let pts = self.samples(svc);
        match self.mode {
            ScopeTrace::Raw => pts,
            ScopeTrace::PeriodAverage {
                high,
                periods,
                fallback,
            } => pts
                .iter()
                .map(|&(t, _)| (t, average_periods(&pts, &pts, t, high, periods, fallback)))
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::service::{ProbeError, Probeable, SamplePlan};
    use crate::signal::Signal;

    struct Pwm(f64);

    impl Probeable for Pwm {
        fn now_secs(&self) -> f64 {
            self.0
        }
        fn sample(&mut self, _: &Signal) -> Result<f64, ProbeError> {
            let phase = (self.0 / 1e-3).fract();
            Ok(if phase < 0.3 { 4e-3 } else { 0.0 })
        }
    }

    fn run() -> (MeasurementService, ChannelId) {
        let mut svc = MeasurementService::new(SamplePlan::default());
        let id = svc.watch_default("i(R2)".parse().unwrap());
        let mut p = Pwm(0.0);
        for k in 0..200 {
            p.0 = k as f64 * 50e-6;
            svc.record(&mut p).unwrap();
        }
        (svc, id)
    }

    #[test]
    fn raw_is_the_ring_in_display_units() {
        let (svc, id) = run();
        let scope = Scope {
            channel: id,
            scale: 1e3,
            mode: ScopeTrace::Raw,
        };
        let want: Vec<_> = svc
            .channel(id)
            .unwrap()
            .ring
            .iter()
            .map(|(t, a)| (t, a * 1e3))
            .collect();
        assert_eq!(scope.trace(&svc), want);
    }

    #[test]
    fn the_pwm_trace_is_the_moving_period_average() {
        let (svc, id) = run();
        let scope = Scope {
            channel: id,
            scale: 1e3,
            mode: ScopeTrace::PeriodAverage {
                high: 0.5,
                periods: 2,
                fallback: 2e-3,
            },
        };
        let pts = scope.samples(&svc);
        let want: Vec<_> = pts
            .iter()
            .map(|&(t, _)| (t, average_periods(&pts, &pts, t, 0.5, 2, 2e-3)))
            .collect();
        let got = scope.trace(&svc);
        assert_eq!(got, want);
        // Late in the run the average sits near the 30 % duty's 1.2 mA (the
        // 50 µs grid catches 6 or 7 of the 20 samples per period high).
        let (_, late) = got[got.len() - 1];
        assert!((1.2..=1.4).contains(&late), "{late}");
    }
}
