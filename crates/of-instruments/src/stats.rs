//! Windowed averages, period-aligned averages and PWM duty/frequency over
//! `(t, value)` samples (moved from `of-ui` in E5a, same maths), plus
//! min/max and time-weighted RMS.

use crate::ring::TRACE_WINDOW;

/// Push `(t, v)` onto a growable vec and drop samples older than [`TRACE_WINDOW`].
/// Prefer [`TraceBuf`](crate::ring::TraceBuf) when the length must stay bounded.
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
    if let Some(&(_, v)) = samples.iter().rev().find(|(t, _)| *t <= start) {
        prev_t = start;
        prev_v = v;
    }
    let mut sum = 0.0;
    let mut used = false;
    for &(t, v) in samples {
        if t < start {
            prev_v = v;
            continue;
        }
        let t_clip = t.min(now);
        let t0 = if used { prev_t } else { start.max(prev_t) };
        if t_clip > t0 {
            sum += prev_v * (t_clip - t0);
            used = true;
        }
        if t >= now {
            prev_t = now;
            break;
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

/// Rising-edge times of a two-level waveform, at or before `now`.
///
/// An edge is recorded at the first sample that crosses `high` from below.
pub fn rising_edges(samples: &[(f64, f64)], now: f64, high: f64) -> Vec<f64> {
    let mut out = Vec::new();
    for w in samples.windows(2) {
        if w[1].0 > now + 1e-15 {
            break;
        }
        if w[0].1 <= high && w[1].1 > high {
            out.push(w[1].0);
        }
    }
    out
}

/// Start and end of the last `periods` complete PWM cycles ending at or before `now`.
pub fn last_period_window(samples: &[(f64, f64)], now: f64, high: f64, periods: usize) -> Option<(f64, f64)> {
    if periods == 0 {
        return None;
    }
    let rises = rising_edges(samples, now, high);
    if rises.len() > periods {
        let t0 = rises[rises.len() - 1 - periods];
        let t1 = rises[rises.len() - 1];
        if t1 - t0 > 1e-15 {
            return Some((t0, t1));
        }
    }
    None
}

/// Time-weighted average over the last `periods` complete PWM cycles.
///
/// The window is a boxcar aligned to rising edges, so a constant-duty PWM
/// averages to a constant (apart from any modelled noise in the samples).
/// If fewer than `periods` cycles have been seen, falls back to
/// [`average_window`] of `fallback` seconds ending at `now`.
///
/// Pass `values` to average a different trace (for example PSU current) over
/// the same LED-derived period window: `edge_samples` locates the edges.
pub fn average_periods(
    values: &[(f64, f64)],
    edge_samples: &[(f64, f64)],
    now: f64,
    high: f64,
    periods: usize,
    fallback: f64,
) -> f64 {
    if let Some((t0, t1)) = last_period_window(edge_samples, now, high, periods) {
        average_window(values, t1, t1 - t0)
    } else {
        average_window(values, now, fallback)
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

/// PWM duty (%) and frequency (Hz) over the whole periods in the last
/// `window` seconds before `now`, for a trace sampled coarser than its edges.
///
/// The threshold is the window's mid level; each crossing is interpolated
/// between its two samples, so rises and falls carry the same bias and a
/// high time keeps its length. Duty is the high time between the first and
/// last rise over their span, and frequency the rises between them over it.
/// `(None, None)` without two rises.
pub fn pwm_duty_freq_periods(samples: &[(f64, f64)], now: f64, window: f64) -> (Option<f64>, Option<f64>) {
    let w: Vec<(f64, f64)> = samples
        .iter()
        .copied()
        .filter(|(t, _)| *t >= now - window && *t <= now + 1e-15)
        .collect();
    let Some((lo, hi)) = w.iter().fold(None, |acc: Option<(f64, f64)>, &(_, v)| {
        Some(acc.map_or((v, v), |(a, b)| (a.min(v), b.max(v))))
    }) else {
        return (None, None);
    };
    if hi - lo <= 0.0 {
        return (None, None);
    }
    let mid = 0.5 * (lo + hi);
    // Interpolated crossings: (t, rising).
    let mut edges = Vec::new();
    for p in w.windows(2) {
        let ((t0, v0), (t1, v1)) = (p[0], p[1]);
        let rising = v0 <= mid && v1 > mid;
        if rising || (v0 > mid && v1 <= mid) {
            edges.push((t0 + (mid - v0) / (v1 - v0) * (t1 - t0), rising));
        }
    }
    let rises: Vec<usize> = (0..edges.len()).filter(|&i| edges[i].1).collect();
    let (Some(&first), Some(&last)) = (rises.first(), rises.last()) else {
        return (None, None);
    };
    if first == last {
        return (None, None);
    }
    let span = edges[last].0 - edges[first].0;
    let mut high = 0.0;
    let mut i = first;
    while i < last {
        // A rise at `i` and the fall after it.
        if edges[i].1 && !edges[i + 1].1 {
            high += edges[i + 1].0 - edges[i].0;
        }
        i += 1;
    }
    let periods = (rises.len() - 1) as f64;
    (Some((high / span * 100.0).clamp(0.0, 100.0)), Some(periods / span))
}

/// [`pwm_duty_freq_periods`] with each edge timed off a `ramp` sampled
/// with `level` (a 555's timing capacitor): the output flips where the
/// ramp turns, so an edge between samples k-1 and k sits where the line
/// through ramp samples k-2, k-1 meets the line through k, k+1 (clamped
/// to the sample interval). Edges without both lines, or a `ramp` not
/// sampled with `level`, fall back to the interpolated crossing.
pub fn pwm_duty_freq_ramp(
    level: &[(f64, f64)],
    ramp: &[(f64, f64)],
    now: f64,
    window: f64,
) -> (Option<f64>, Option<f64>) {
    let inside = |pts: &[(f64, f64)]| -> Vec<(f64, f64)> {
        pts.iter()
            .copied()
            .filter(|(t, _)| *t >= now - window && *t <= now + 1e-15)
            .collect()
    };
    let (w, r) = (inside(level), inside(ramp));
    let paired = w.len() == r.len() && w.iter().zip(&r).all(|(a, b)| (a.0 - b.0).abs() <= 1e-12);
    let Some(mid) = mid_level(&w) else {
        return (None, None);
    };
    let mut edges = Vec::new();
    for k in 1..w.len() {
        let ((t0, v0), (t1, v1)) = (w[k - 1], w[k]);
        let rising = v0 <= mid && v1 > mid;
        if !(rising || (v0 > mid && v1 <= mid)) {
            continue;
        }
        let mut t = t0 + (mid - v0) / (v1 - v0) * (t1 - t0);
        if paired && k >= 2 && k + 1 < r.len() {
            let sa = (r[k - 1].1 - r[k - 2].1) / (r[k - 1].0 - r[k - 2].0);
            let sb = (r[k + 1].1 - r[k].1) / (r[k + 1].0 - r[k].0);
            if (sa - sb).abs() > 1e-12 {
                // r[k-1] + sa (x - t0) = r[k] + sb (x - t1)
                let x = (r[k].1 - r[k - 1].1 + sa * t0 - sb * t1) / (sa - sb);
                if x.is_finite() {
                    t = x.clamp(t0, t1);
                }
            }
        }
        edges.push((t, rising));
    }
    duty_freq_from_edges(&edges)
}

/// The mid level of `w` (None for a flat or empty trace).
fn mid_level(w: &[(f64, f64)]) -> Option<f64> {
    let (lo, hi) = w.iter().fold(None, |acc: Option<(f64, f64)>, &(_, v)| {
        Some(acc.map_or((v, v), |(a, b)| (a.min(v), b.max(v))))
    })?;
    (hi - lo > 0.0).then_some(0.5 * (lo + hi))
}

/// Duty (%) and frequency (Hz) between the first and last rise of `edges`
/// (`(t, rising)`, in time order).
fn duty_freq_from_edges(edges: &[(f64, bool)]) -> (Option<f64>, Option<f64>) {
    let rises: Vec<usize> = (0..edges.len()).filter(|&i| edges[i].1).collect();
    let (Some(&first), Some(&last)) = (rises.first(), rises.last()) else {
        return (None, None);
    };
    if first == last {
        return (None, None);
    }
    let span = edges[last].0 - edges[first].0;
    let high: f64 = (first..last)
        .filter(|&i| edges[i].1 && !edges[i + 1].1)
        .map(|i| edges[i + 1].0 - edges[i].0)
        .sum();
    let periods = (rises.len() - 1) as f64;
    (Some((high / span * 100.0).clamp(0.0, 100.0)), Some(periods / span))
}

/// Smallest and largest value of the samples at or after `from`.
pub fn min_max(samples: &[(f64, f64)], from: f64) -> Option<(f64, f64)> {
    samples
        .iter()
        .filter(|(t, _)| *t >= from)
        .fold(None, |acc, &(_, v)| match acc {
            None => Some((v, v)),
            Some((lo, hi)) => Some((f64::min(lo, v), f64::max(hi, v))),
        })
}

/// Time-weighted RMS from `from` to `now`. Each sample holds until the next
/// one (as the scope draws it); the value in force at `from` counts from
/// `from`. `None` when no sample is at or before `now` or the span is empty.
pub fn rms(samples: &[(f64, f64)], from: f64, now: f64) -> Option<f64> {
    if now <= from {
        return None;
    }
    let mut level = samples.iter().rev().find(|(t, _)| *t <= from).map(|&(_, v)| v);
    let mut at = from;
    let mut sum_sq = 0.0;
    let mut covered = 0.0;
    for &(t, v) in samples.iter().filter(|(t, _)| *t > from && *t <= now) {
        if let Some(l) = level {
            sum_sq += l * l * (t - at);
            covered += t - at;
        }
        level = Some(v);
        at = t;
    }
    let l = level?;
    sum_sq += l * l * (now - at);
    covered += now - at;
    (covered > 0.0).then(|| (sum_sq / covered).sqrt())
}

#[cfg(test)]
mod tests {
    /// An 18 % PWM of period 1.0458 ms (the 555 dimmer at pot 0.1) on the
    /// 50 us grid, with its timing ramp: the ramp-timed edges read within
    /// 0.05 points and 0.1 %.
    #[test]
    fn ramp_timed_edges_read_a_short_pulse_on_a_coarse_grid() {
        let (period, duty) = (1.0458e-3, 0.18);
        let (lo, hi) = (5.0 / 3.0, 10.0 / 3.0);
        let pts: Vec<(f64, f64, f64)> = (0..=400)
            .map(|k| {
                let t = f64::from(k) * 50e-6;
                let ph = (t / period).fract();
                let (on, cap) = if ph < duty {
                    (9.47, lo + (hi - lo) * ph / duty)
                } else {
                    (0.0, hi - (hi - lo) * (ph - duty) / (1.0 - duty))
                };
                (t, on, cap)
            })
            .collect();
        let level: Vec<(f64, f64)> = pts.iter().map(|&(t, v, _)| (t, v)).collect();
        let ramp: Vec<(f64, f64)> = pts.iter().map(|&(t, _, c)| (t, c)).collect();
        let (d, f) = pwm_duty_freq_ramp(&level, &ramp, 20e-3, 20e-3);
        let (d, f) = (d.unwrap(), f.unwrap());
        assert!((d - duty * 100.0).abs() < 0.05, "duty {d}");
        assert!((f * period - 1.0).abs() < 1e-3, "freq {f}");
        // The plain crossing reads it about a point off.
        let (p, _) = pwm_duty_freq_periods(&level, 20e-3, 20e-3);
        assert!((p.unwrap() - duty * 100.0).abs() > 0.3, "{p:?}");
    }

    /// A 52.99 % PWM of period 1.0328 ms (the 555 dimmer at mid pot) on the
    /// 50 us grid reads within 0.5 points and 1 %.
    #[test]
    fn whole_periods_with_interpolated_edges_read_a_coarse_pwm() {
        let (period, duty) = (1.0328e-3, 0.5299);
        let pts: Vec<(f64, f64)> = (0..=400)
            .map(|k| {
                let t = f64::from(k) * 50e-6;
                let on = (t / period).fract() < duty;
                (t, if on { 9.47 } else { 0.0 })
            })
            .collect();
        let (d, f) = pwm_duty_freq_periods(&pts, 20e-3, 20e-3);
        let (d, f) = (d.unwrap(), f.unwrap());
        assert!((d - duty * 100.0).abs() < 0.5, "duty {d}");
        assert!((f * period - 1.0).abs() < 0.01, "freq {f}");
        // A flat trace or one period has no reading.
        assert_eq!(pwm_duty_freq_periods(&[(0.0, 1.0), (1.0, 1.0)], 1.0, 2.0), (None, None));
    }

    use super::*;

    /// 1 kHz square wave, `high` for `duty` of each period, sampled at the edges.
    fn square(high: f64, duty: f64, periods: usize) -> Vec<(f64, f64)> {
        let mut s = Vec::new();
        for k in 0..periods {
            let t0 = k as f64 * 1e-3;
            s.push((t0, high));
            s.push((t0 + duty * 1e-3, 0.0));
        }
        s
    }

    #[test]
    fn min_max_covers_only_the_window() {
        let s = [(0.0, 5.0), (1.0, -1.0), (2.0, 2.0), (3.0, 3.0)];
        assert_eq!(min_max(&s, 0.0), Some((-1.0, 5.0)));
        assert_eq!(min_max(&s, 1.5), Some((2.0, 3.0)));
        assert_eq!(min_max(&s, 9.0), None);
        assert_eq!(min_max(&[], 0.0), None);
    }

    #[test]
    fn rms_of_a_square_wave_is_high_times_root_duty() {
        let s = square(4.0, 0.5, 10);
        let r = rms(&s, 0.0, 10e-3).unwrap();
        assert!((r - 4.0 * 0.5f64.sqrt()).abs() < 1e-12, "{r}");
        let s = square(4.0, 0.25, 10);
        let r = rms(&s, 0.0, 10e-3).unwrap();
        assert!((r - 2.0).abs() < 1e-12, "{r}");
    }

    #[test]
    fn rms_of_dc_is_its_magnitude_and_uses_the_value_in_force() {
        let s = [(0.0, -3.0)];
        assert!((rms(&s, 0.5, 2.0).unwrap() - 3.0).abs() < 1e-15);
        // A window that starts between samples holds the earlier value.
        let s = [(0.0, 1.0), (1.0, 3.0)];
        let r = rms(&s, 0.5, 1.5).unwrap();
        assert!((r - 5.0f64.sqrt()).abs() < 1e-12, "{r}");
        assert_eq!(rms(&s, 1.0, 1.0), None);
        assert_eq!(rms(&[(2.0, 1.0)], 0.0, 1.0), None);
    }

    #[test]
    fn period_mean_matches_duty_times_high() {
        // The moved maths, checked from this crate.
        let mut s = square(4.0, 0.65, 20);
        s.push((20e-3, 4.0));
        let m = average_periods(&s, &s, 20e-3, 0.5, 2, 0.002);
        assert!((m - 4.0 * 0.65).abs() < 1e-9, "{m}");
        let (duty, freq) = pwm_duty_freq(&s, 20e-3, 0.5);
        assert!((duty.unwrap() - 65.0).abs() < 1e-6);
        assert!((freq.unwrap() - 1000.0).abs() < 1e-6);
    }
}
