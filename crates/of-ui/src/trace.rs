//! Rolling samples, windowed averages, and PWM duty/frequency from a trace.

/// 1-2-5 time/div steps for the live LED-current scope (seconds per division).
pub const TIME_DIVS: &[f64] = &[
    10e-6, 20e-6, 50e-6, 100e-6, 200e-6, 500e-6, 1e-3, 2e-3, 5e-3, 10e-3, 20e-3, 50e-3, 100e-3, 200e-3, 500e-3, 1.0,
];
/// Horizontal divisions on the live scope (matches the existing 5 tick intervals).
pub const SCOPE_DIVISIONS: f64 = 5.0;
/// Default time/div (20 ms): a 100 ms visible window, close to the previous 120 ms span.
pub const DEFAULT_TIME_DIV: f64 = 20e-3;
/// Finest scope recording interval (seconds). PWM edges still occur at their exact
/// simulated time; the analog solver's `max_step` is 5 µs, so continuous ramps are
/// not resolved finer than that. The scope may sample down to 1 µs when zoomed in.
pub const MIN_SAMPLE_STEP: f64 = 1e-6;
/// Hard cap on ring length so a 5 s window cannot grow without bound.
pub const MAX_TRACE_SAMPLES: usize = 32_768;
/// Default spacing of recorded samples (seconds) when the window fits it.
pub const SAMPLE_STEP: f64 = 50e-6;
/// Window used for the LED current readout (seconds) when a PWM period is not yet known.
pub const LED_AVG_WINDOW: f64 = 0.002;
/// Complete PWM cycles in the period-aligned display average (two 1 kHz periods, ten 5 kHz).
pub const PWM_AVG_PERIODS: usize = 2;
/// Instantaneous current (mA) that counts as PWM high when locating period edges.
pub const PWM_EDGE_HIGH: f64 = 0.5;
/// `pwm_duty_freq` looks at the last 20 ms, so retention never shrinks below this.
pub const MIN_RETENTION: f64 = 0.02;
/// Margin past the left edge before a sample may be dropped (one division).
const RETENTION_MARGIN_DIVS: f64 = 1.0;

/// Default visible plot span (seconds): [`SCOPE_DIVISIONS`] × [`DEFAULT_TIME_DIV`].
pub const DEFAULT_VISIBLE_WINDOW: f64 = SCOPE_DIVISIONS * DEFAULT_TIME_DIV;

/// Default retention used by [`TraceBuf::default_scope`] (visible window + margin,
/// and at least [`MIN_RETENTION`]). Kept as `TRACE_WINDOW` for existing call sites.
pub const TRACE_WINDOW: f64 = DEFAULT_VISIBLE_WINDOW + DEFAULT_TIME_DIV * RETENTION_MARGIN_DIVS;

/// Visible plot span for a time/div setting (seconds).
pub fn visible_window(time_div: f64) -> f64 {
    time_div.max(TIME_DIVS[0]) * SCOPE_DIVISIONS
}

/// How long the ring keeps samples: the visible window, one extra division of
/// margin, and at least [`MIN_RETENTION`] so averages and PWM readouts still work
/// when the plot is zoomed in.
pub fn retention_window(visible: f64) -> f64 {
    let visible = visible.max(0.0);
    let margin = visible / SCOPE_DIVISIONS * RETENTION_MARGIN_DIVS;
    (visible + margin).max(MIN_RETENTION)
}

/// Recording step for a visible span and its retention window.
///
/// Prefers [`SAMPLE_STEP`] when that already fills the view (at least 32 points)
/// and fits in [`MAX_TRACE_SAMPLES`]. Zooms in go down to [`MIN_SAMPLE_STEP`];
/// wide windows get a coarser step so the ring stays capped.
pub fn sample_step_for_window(visible: f64, retain: f64) -> f64 {
    let cap_step = retain.max(MIN_SAMPLE_STEP) / (MAX_TRACE_SAMPLES.saturating_sub(8) as f64);
    let min_step = MIN_SAMPLE_STEP.max(cap_step);
    let preferred = if visible / SAMPLE_STEP >= 32.0 {
        SAMPLE_STEP
    } else {
        (visible / 1000.0).max(MIN_SAMPLE_STEP)
    };
    preferred.max(min_step)
}

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

fn cap_for(window: f64, sample_step: f64) -> usize {
    let step = if sample_step > 0.0 { sample_step } else { SAMPLE_STEP };
    let needed = ((window.max(0.0) / step).ceil() as usize).saturating_add(8).max(16);
    needed.min(MAX_TRACE_SAMPLES)
}

/// Fixed-capacity ring of `(t, value)` samples for the live scope.
///
/// Memory is bounded by [`TraceBuf::capacity`], which is derived from the
/// time window and sample step, and never exceeds [`MAX_TRACE_SAMPLES`].
/// Samples older than the window are dropped, and if the ring is full the
/// oldest sample is overwritten, so a long run cannot grow the buffer.
#[derive(Clone, Debug)]
pub struct TraceBuf {
    buf: Vec<(f64, f64)>,
    cap: usize,
    start: usize,
    len: usize,
    window: f64,
    sample_step: f64,
}

impl TraceBuf {
    /// A buffer that holds `window` seconds at `sample_step` spacing.
    pub fn with_window(window: f64, sample_step: f64) -> Self {
        let window = window.max(0.0);
        let step = if sample_step > 0.0 { sample_step } else { SAMPLE_STEP };
        let cap = cap_for(window, step);
        Self {
            buf: vec![(0.0, 0.0); cap],
            cap,
            start: 0,
            len: 0,
            window,
            sample_step: step,
        }
    }

    /// Default visible-window retention at [`SAMPLE_STEP`].
    pub fn default_scope() -> Self {
        Self::with_window(TRACE_WINDOW, SAMPLE_STEP)
    }

    /// A ring sized for this visible plot span (retention + sample step included).
    #[allow(dead_code)]
    pub fn for_visible(visible: f64) -> Self {
        let retain = retention_window(visible);
        Self::with_window(retain, sample_step_for_window(visible, retain))
    }

    /// Configured time window (seconds).
    #[allow(dead_code)]
    pub fn window(&self) -> f64 {
        self.window
    }

    /// Current recording step (seconds). The caller decides when to `push`.
    pub fn sample_step(&self) -> f64 {
        self.sample_step
    }

    /// Change the visible plot span: updates retention and the preferred sample step.
    /// Capacity may grow or shrink, but never past [`MAX_TRACE_SAMPLES`].
    pub fn set_visible_window(&mut self, visible: f64) {
        let retain = retention_window(visible);
        let step = sample_step_for_window(visible, retain);
        self.set_retention(retain, step);
    }

    /// Change the time window and sample step. Capacity may be reallocated,
    /// but never above [`MAX_TRACE_SAMPLES`].
    pub fn set_retention(&mut self, window: f64, sample_step: f64) {
        let window = window.max(0.0);
        let step = if sample_step > 0.0 { sample_step } else { SAMPLE_STEP };
        let cap = cap_for(window, step);
        self.window = window;
        self.sample_step = step;
        if cap != self.cap {
            let pts: Vec<(f64, f64)> = self.iter().collect();
            *self = Self::with_window(window, step);
            for (t, v) in pts {
                self.push(t, v);
            }
        } else if let Some((t, _)) = self.last() {
            self.trim(t);
        }
    }

    /// Change the time window. Capacity is recomputed for the current step.
    #[allow(dead_code)]
    pub fn set_window(&mut self, window: f64) {
        self.set_retention(window, self.sample_step);
    }

    /// Maximum number of samples this buffer will hold.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Number of samples currently stored.
    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    pub fn last(&self) -> Option<(f64, f64)> {
        if self.len == 0 {
            None
        } else {
            Some(self.buf[(self.start + self.len - 1) % self.cap])
        }
    }

    /// Push `(t, v)` and drop samples older than the window or past capacity.
    pub fn push(&mut self, t: f64, v: f64) {
        if self.len == self.cap {
            self.start = (self.start + 1) % self.cap;
            self.len -= 1;
        }
        let i = (self.start + self.len) % self.cap;
        self.buf[i] = (t, v);
        self.len += 1;
        self.trim(t);
    }

    fn trim(&mut self, t: f64) {
        let start_t = t - self.window;
        while self.len > 1 {
            if self.buf[self.start].0 >= start_t {
                break;
            }
            self.start = (self.start + 1) % self.cap;
            self.len -= 1;
        }
    }

    pub fn iter(&self) -> impl Iterator<Item = (f64, f64)> + '_ {
        let cap = self.cap;
        let start = self.start;
        let buf = &self.buf;
        (0..self.len).map(move |k| buf[(start + k) % cap])
    }

    /// Samples at or after `start` (for a snapshot of the visible plot).
    #[allow(dead_code)]
    pub fn iter_from(&self, start: f64) -> impl Iterator<Item = (f64, f64)> + '_ {
        self.iter().filter(move |(t, _)| *t >= start)
    }

    /// Copy the samples into `out` (reusing its allocation).
    #[allow(dead_code)]
    pub fn fill_vec(&self, out: &mut Vec<(f64, f64)>) {
        out.clear();
        out.extend(self.iter());
    }

    /// Owned copy, for snapshots and helpers that still take a slice.
    pub fn to_vec(&self) -> Vec<(f64, f64)> {
        self.iter().collect()
    }
}

impl Default for TraceBuf {
    fn default() -> Self {
        Self::default_scope()
    }
}

impl std::ops::Index<usize> for TraceBuf {
    type Output = (f64, f64);

    fn index(&self, i: usize) -> &Self::Output {
        assert!(i < self.len, "index {i} >= len {}", self.len);
        &self.buf[(self.start + i) % self.cap]
    }
}

/// Push `(t, v)` onto a growable vec and drop samples older than [`TRACE_WINDOW`].
/// Prefer [`TraceBuf`] when the length must stay bounded.
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

/// When to record the next scope sample on a fixed time grid.
///
/// The live worker may advance in slices much smaller than the sample step
/// (especially at very low playback speeds). Recording on every slice would
/// pack the ring and evict history that is still on the plot. Call
/// [`SampleClock::take`] only when it returns `true`.
#[derive(Clone, Debug)]
pub struct SampleClock {
    next: f64,
    step: f64,
}

impl SampleClock {
    pub fn new(start: f64, step: f64) -> Self {
        Self {
            next: start,
            step: if step > 0.0 { step } else { SAMPLE_STEP },
        }
    }

    #[allow(dead_code)]
    pub fn step(&self) -> f64 {
        self.step
    }

    pub fn next(&self) -> f64 {
        self.next
    }

    /// Update the grid. A finer step may pull `next` closer so zoom-in takes effect promptly.
    pub fn set_step(&mut self, step: f64, now: f64) {
        let step = if step > 0.0 { step } else { SAMPLE_STEP };
        self.step = step;
        if self.next > now + step {
            self.next = now + step;
        }
    }

    /// If `now` has reached the next grid time, consume it (advancing the grid) and return true.
    pub fn take(&mut self, now: f64) -> bool {
        if now + 1e-15 < self.next {
            return false;
        }
        while self.next <= now + 1e-15 {
            self.next += self.step;
            if !self.next.is_finite() {
                self.next = now + self.step;
                break;
            }
        }
        true
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
