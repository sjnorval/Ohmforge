//! Rolling samples, windowed averages, and PWM duty/frequency from a trace.

/// How much history the live scope and averages keep (seconds).
pub const TRACE_WINDOW: f64 = 0.16;
/// Default spacing of recorded samples (seconds).
pub const SAMPLE_STEP: f64 = 50e-6;
/// Window used for the LED current readout (seconds).
pub const LED_AVG_WINDOW: f64 = 0.002;

/// Push `(t, v)` and drop samples older than [`TRACE_WINDOW`] before `t`.
#[allow(dead_code)]
pub fn push_sample(samples: &mut Vec<(f64, f64)>, t: f64, v: f64) {
    samples.push((t, v));
    let start = t - TRACE_WINDOW;
    if let Some(i) = samples.iter().position(|(st, _)| *st >= start) {
        if i > 0 {
            samples.drain(0..i);
        }
    } else {
        samples.clear();
    }
}

/// Time-weighted average of `samples` over the last `window` seconds ending at `now`.
pub fn average_window(samples: &[(f64, f64)], now: f64, window: f64) -> f64 {
    if samples.is_empty() {
        return 0.0;
    }
    let start = now - window;
    let mut prev_t = start;
    let mut prev_v = samples[0].1;
    for &(t, v) in samples {
        if t < start {
            prev_v = v;
            continue;
        }
        prev_t = start;
        prev_v = v;
        break;
    }
    if let Some(&(t, v)) = samples.iter().rev().find(|(t, _)| *t <= start) {
        prev_t = start;
        prev_v = v;
        let _ = (t, v);
    }
    let mut sum = 0.0;
    let mut used = false;
    for &(t, v) in samples {
        if t < start {
            prev_v = v;
            continue;
        }
        let t0 = if used { prev_t } else { start.max(prev_t) };
        if t > t0 {
            sum += prev_v * (t - t0);
            used = true;
        }
        prev_t = t;
        prev_v = v;
    }
    if now > prev_t {
        sum += prev_v * (now - prev_t);
        used = true;
    }
    let span = if used { (now - start).max(1e-15) } else { 0.0 };
    if span > 0.0 {
        sum / span
    } else {
        samples.last().map(|s| s.1).unwrap_or(0.0)
    }
}

/// PWM duty cycle (%) and frequency (Hz) from a voltage (or current) trace.
///
/// `high` is the threshold that counts as the on level. Looks at the last
/// 20 ms of samples, or the last complete period if one is visible.
pub fn pwm_duty_freq(samples: &[(f64, f64)], now: f64, high: f64) -> (Option<f64>, Option<f64>) {
    let lo = now - 0.02;
    let window: Vec<(f64, f64)> = samples
        .iter()
        .copied()
        .filter(|(t, _)| *t >= lo && *t <= now + 1e-15)
        .collect();
    if window.len() < 2 {
        return (None, None);
    }
    let mut high_t = 0.0;
    let mut last_rise = None;
    let mut period = None;
    for w in window.windows(2) {
        let dt = w[1].0 - w[0].0;
        let was = w[0].1 > high;
        if was {
            high_t += dt;
        }
        if !was && w[1].1 > high {
            if let Some(t0) = last_rise {
                period = Some(w[1].0 - t0);
            }
            last_rise = Some(w[1].0);
        }
    }
    let span = window.last().unwrap().0 - window.first().unwrap().0;
    let duty = if span > 0.0 {
        Some((high_t / span * 100.0).clamp(0.0, 100.0))
    } else {
        None
    };
    let freq = period.filter(|p| *p > 1e-9).map(|p| 1.0 / p);
    (duty, freq)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn average_and_pwm_from_a_square_wave() {
        let mut s = Vec::new();
        // 1 kHz, 25 % duty, 1 V high, from t = 0 to 20 ms.
        for i in 0..=2000 {
            let t = i as f64 * 10e-6;
            let phase = t % 1e-3;
            let v = if phase < 0.25e-3 { 1.0 } else { 0.0 };
            push_sample(&mut s, t, v);
        }
        let now = 0.02;
        let avg = average_window(&s, now, 0.002);
        assert!((avg - 0.25).abs() < 0.02, "{avg}");
        let (duty, freq) = pwm_duty_freq(&s, now, 0.5);
        assert!((duty.unwrap() - 25.0).abs() < 1.5, "{duty:?}");
        assert!((freq.unwrap() - 1000.0).abs() < 20.0, "{freq:?}");
        assert!(s[0].0 >= now - TRACE_WINDOW);
    }
}
