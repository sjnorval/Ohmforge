//! Scope time/div steps and display formatting. The sample ring, sample
//! clock and statistics live in `of_instruments` (engine E5a) and are
//! re-exported here so existing call sites keep working.

pub use of_instruments::ring::*;
pub use of_instruments::stats::*;

/// Snap `time_div` to the nearest entry in [`TIME_DIVS`].
pub fn snap_time_div(time_div: f64) -> f64 {
    let mut best = TIME_DIVS[0];
    let mut best_d = (time_div - best).abs();
    for &d in TIME_DIVS.iter().skip(1) {
        let err = (time_div - d).abs();
        if err < best_d {
            best = d;
            best_d = err;
        }
    }
    best
}

/// Move `steps` along [`TIME_DIVS`] (negative zooms in). Clamped at the ends.
pub fn step_time_div(time_div: f64, steps: i32) -> f64 {
    let cur = snap_time_div(time_div);
    let i = TIME_DIVS.iter().position(|&d| d == cur).unwrap_or(0);
    let next = i as i32 + steps;
    TIME_DIVS[next.clamp(0, TIME_DIVS.len() as i32 - 1) as usize]
}

/// Format a time/div value, e.g. `20 ms/div`.
pub fn format_time_div(div: f64) -> String {
    format_seconds(div, "/div")
}

/// Format a playback or realtime multiplier, keeping small values readable.
pub fn format_multiplier(x: f64) -> String {
    if x >= 0.01 {
        format!("{x:.2}×")
    } else if x >= 1e-4 {
        format!("{x:.4}×")
    } else {
        format!("{x:.1e}×")
    }
}

/// Tick or caption for a time in seconds, with a unit that fits `span`.
pub fn format_axis_time(t: f64, span: f64) -> (String, &'static str) {
    if span < 1e-3 {
        (format!("{:.1}", t * 1e6), "µs")
    } else if span < 1.0 {
        let dec = if span < 10e-3 {
            3
        } else if span < 0.1 {
            2
        } else {
            1
        };
        (format!("{:.*}", dec, t * 1e3), "ms")
    } else {
        (format!("{t:.3}"), "s")
    }
}

fn format_seconds(t: f64, suffix: &str) -> String {
    if t < 1e-3 {
        format!("{} µs{suffix}", nice_num(t * 1e6))
    } else if t < 1.0 {
        format!("{} ms{suffix}", nice_num(t * 1e3))
    } else {
        format!("{} s{suffix}", nice_num(t))
    }
}

fn nice_num(x: f64) -> String {
    if (x - x.round()).abs() < 1e-9 {
        format!("{}", x.round() as i64)
    } else {
        let s = format!("{x:.6}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn period_aligned_average_is_flat_at_constant_duty() {
        // 1 kHz, 65 % duty, 4 mA high, sampled every 50 us and at exact edges.
        let duty: f64 = 0.65;
        let period: f64 = 1e-3;
        let on: f64 = 4.0;
        let mut pts: Vec<(f64, f64)> = Vec::new();
        let end: f64 = 0.08;
        let mut cycle: f64 = 0.0;
        pts.push((0.0, on));
        while cycle < end {
            let t_fall = cycle + duty * period;
            let t_rise = cycle + period;
            let mut g = cycle + 50e-6;
            while g < t_rise - 1e-12 && g <= end {
                if (g - t_fall).abs() > 1e-12 {
                    pts.push((g, if g < t_fall { on } else { 0.0 }));
                }
                g += 50e-6;
            }
            if t_fall <= end {
                pts.push((t_fall, 0.0));
            }
            if t_rise <= end {
                pts.push((t_rise, on));
            }
            cycle = t_rise;
        }
        pts.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut avgs = Vec::new();
        for &(t, _) in &pts {
            if t < 0.01 {
                continue;
            }
            avgs.push(average_periods(
                &pts,
                &pts,
                t,
                PWM_EDGE_HIGH,
                PWM_AVG_PERIODS,
                LED_AVG_WINDOW,
            ));
        }
        assert!(!avgs.is_empty());
        let mean = avgs.iter().sum::<f64>() / avgs.len() as f64;
        let peak = avgs.iter().fold(0.0_f64, |m, v| m.max((v - mean).abs()));
        assert!(
            (mean - on * duty).abs() < 0.02,
            "mean {mean} mA, expected {}",
            on * duty
        );
        assert!(
            peak < 0.02,
            "period-aligned ripple {peak} mA around {mean} (bound 0.02)"
        );
    }

    #[test]
    fn average_and_pwm_from_a_square_wave() {
        let mut s = TraceBuf::default_scope();
        // 1 kHz, 25 % duty, 1 V high, from t = 0 to 20 ms.
        for i in 0..=2000 {
            let t = i as f64 * 10e-6;
            let phase = t % 1e-3;
            let v = if phase < 0.25e-3 { 1.0 } else { 0.0 };
            s.push(t, v);
        }
        let now = 0.02;
        let pts = s.to_vec();
        let avg = average_window(&pts, now, 0.002);
        assert!((avg - 0.25).abs() < 0.02, "{avg}");
        let (duty, freq) = pwm_duty_freq(&pts, now, 0.5);
        assert!((duty.unwrap() - 25.0).abs() < 1.5, "{duty:?}");
        assert!((freq.unwrap() - 1000.0).abs() < 20.0, "{freq:?}");
        assert!(s.iter().next().unwrap().0 >= now - TRACE_WINDOW);
    }

    #[test]
    fn ring_is_capped_and_wraps() {
        let mut s = TraceBuf::with_window(0.01, 1e-3);
        assert!(s.capacity() < 32, "cap {}", s.capacity());
        for i in 0..10_000 {
            s.push(i as f64 * 1e-3, i as f64);
            assert!(s.len() <= s.capacity());
        }
        // The time window (10 ms at 1 ms spacing) is smaller than the slack
        // capacity, so the ring is not completely full.
        assert!(s.len() <= s.capacity());
        assert!(s.len() >= 8, "window under-filled: {}", s.len());
        let pts: Vec<_> = s.iter().collect();
        assert!(pts.windows(2).all(|w| w[1].0 >= w[0].0));
        let last_t = pts.last().unwrap().0;
        assert!((last_t - 9.999).abs() < 1e-9, "{last_t}");
        assert!(pts[0].0 >= last_t - s.window() - 1e-12);
    }

    #[test]
    fn long_run_memory_stays_at_capacity() {
        let mut s = TraceBuf::default_scope();
        let cap = s.capacity();
        // Several simulated minutes at the default sample spacing.
        let minutes = 3.0;
        let n = (minutes * 60.0 / SAMPLE_STEP).ceil() as usize;
        for i in 0..=n {
            s.push(i as f64 * SAMPLE_STEP, (i % 7) as f64);
            assert!(s.len() <= cap, "len {} cap {cap} at sample {i}", s.len());
        }
        assert_eq!(s.capacity(), cap);
        assert!(s.len() <= cap);
        assert!(s.len() > cap / 2, "window under-filled: {} / {cap}", s.len());
    }

    #[test]
    fn time_divs_are_1_2_5_from_10us_to_1s() {
        assert_eq!(TIME_DIVS.first().copied(), Some(10e-6));
        assert_eq!(TIME_DIVS.last().copied(), Some(1.0));
        for w in TIME_DIVS.windows(2) {
            assert!(w[1] > w[0], "{} then {}", w[0], w[1]);
        }
        let mantissa = |d: f64| {
            let mut x = d;
            while x >= 10.0 {
                x /= 10.0;
            }
            while x < 1.0 {
                x *= 10.0;
            }
            x
        };
        for &d in TIME_DIVS {
            let m = mantissa(d);
            assert!(
                (m - 1.0).abs() < 1e-9 || (m - 2.0).abs() < 1e-9 || (m - 5.0).abs() < 1e-9,
                "not 1-2-5: {d} (mantissa {m})"
            );
        }
        assert_eq!(snap_time_div(24e-3), 20e-3);
        assert_eq!(snap_time_div(0.0), 10e-6);
        assert_eq!(step_time_div(20e-3, -1), 10e-3);
        assert_eq!(step_time_div(10e-6, -3), 10e-6);
        assert_eq!(step_time_div(1.0, 2), 1.0);
        assert_eq!(step_time_div(20e-3, 1), 50e-3);
    }

    #[test]
    fn retention_covers_the_visible_window_plus_margin() {
        let visible = visible_window(20e-3);
        assert!((visible - 0.1).abs() < 1e-15);
        let retain = retention_window(visible);
        // One extra division (20 ms) past the left edge.
        assert!((retain - 0.12).abs() < 1e-15, "{retain}");
        // Zoomed in: still keep 20 ms for PWM / LED averages.
        let tight = retention_window(visible_window(10e-6));
        assert!(tight >= MIN_RETENTION - 1e-15, "{tight}");
        assert!(tight >= visible_window(10e-6));
    }

    #[test]
    fn samples_stay_until_they_leave_the_visible_plot() {
        let time_div = 20e-3;
        let visible = visible_window(time_div);
        let mut s = TraceBuf::for_visible(visible);
        let step = s.sample_step();
        let now = 1.0;
        let n = (now / step).ceil() as usize;
        for i in 0..=n {
            s.push(i as f64 * step, 1.0);
        }
        let pts: Vec<_> = s.iter().collect();
        let last = pts.last().unwrap().0;
        let left = last - visible;
        // Every sample still on the plot is present.
        assert!(
            pts.iter()
                .any(|(t, _)| (*t - left).abs() < step * 1.5 || *t <= left + step),
            "left edge {left} missing; first {}",
            pts[0].0
        );
        for &(t, _) in &pts {
            assert!(t >= last - s.window() - 1e-12);
        }
        // Nothing still on-screen was dropped: the oldest kept sample is at or
        // before the left edge (plus a tiny grid slack).
        assert!(
            pts[0].0 <= left + step + 1e-12,
            "cleared too early: first {} left {left} step {step}",
            pts[0].0
        );
    }

    #[test]
    fn wide_window_is_capped_and_decimated() {
        let visible = visible_window(1.0); // 5 s
        let retain = retention_window(visible);
        let step = sample_step_for_window(visible, retain);
        assert!(step > SAMPLE_STEP, "expected decimation, step {step}");
        let mut s = TraceBuf::for_visible(visible);
        assert!(s.capacity() <= MAX_TRACE_SAMPLES);
        let n = (retain / step).ceil() as usize + 100;
        for i in 0..=n * 3 {
            s.push(i as f64 * step, 0.0);
            assert!(s.len() <= MAX_TRACE_SAMPLES);
        }
        assert!(s.len() <= s.capacity());
        assert!(s.window() >= visible);
    }

    #[test]
    fn zoomed_in_uses_finer_sampling() {
        let visible = visible_window(10e-6);
        let retain = retention_window(visible);
        let step = sample_step_for_window(visible, retain);
        assert!((step - MIN_SAMPLE_STEP).abs() < 1e-15, "step {step}");
        assert!(visible / step >= 32.0, "too few points in view: {}", visible / step);
    }

    #[test]
    fn set_visible_window_keeps_samples_in_the_new_view() {
        let mut s = TraceBuf::for_visible(0.1);
        for i in 0..=4000 {
            s.push(i as f64 * SAMPLE_STEP, i as f64);
        }
        let now = s.last().unwrap().0;
        let before: Vec<f64> = s.iter().map(|(t, _)| t).collect();
        s.set_visible_window(visible_window(50e-3)); // 250 ms view: we cannot invent older history
        let pts: Vec<_> = s.iter().collect();
        // Every previously stored sample is still present (new retention is larger).
        for t0 in before {
            assert!(
                pts.iter().any(|(t, _)| (*t - t0).abs() < 1e-12),
                "lost sample {t0} when zooming out; first {} last {now}",
                pts[0].0
            );
        }
        assert!(pts.iter().any(|(t, _)| *t >= now - 0.1));
    }

    #[test]
    fn sample_clock_skips_tiny_slices() {
        // Without a grid, 32_768 pushes 1 µs apart fill the cap in 32 ms and
        // evict a 100 ms view. The clock records once per 50 µs instead.
        let visible = 0.1;
        let mut s = TraceBuf::for_visible(visible);
        let mut clock = SampleClock::new(0.0, SAMPLE_STEP);
        let end = 0.25;
        let mut t = 0.0;
        let dt = 1e-6;
        while t <= end {
            if clock.take(t) {
                s.push(t, 1.0);
            }
            t += dt;
        }
        let pts: Vec<_> = s.iter().collect();
        let last = pts.last().unwrap().0;
        let left = last - visible;
        assert!(
            pts[0].0 <= left + SAMPLE_STEP * 2.0,
            "cleared too early at slow pacing: first {} left {left}",
            pts[0].0
        );
        let on_screen = pts.iter().filter(|(t, _)| *t >= left - 1e-12).count();
        assert!(on_screen > 100, "sparse on-screen trace: {on_screen}");
        assert!(s.len() <= s.capacity());
    }

    #[test]
    fn format_helpers_cover_small_values() {
        assert_eq!(format_time_div(20e-3), "20 ms/div");
        assert_eq!(format_time_div(10e-6), "10 µs/div");
        assert_eq!(format_time_div(1.0), "1 s/div");
        assert_eq!(format_multiplier(1.0), "1.00×");
        assert_eq!(format_multiplier(1e-6), "1.0e-6×");
        let (tick, unit) = format_axis_time(1.234e-3, 50e-6);
        assert_eq!(unit, "µs");
        assert!(tick.contains("1234") || tick.contains("1.2"));
    }
}
