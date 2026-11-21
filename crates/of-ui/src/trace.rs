//! Rolling samples, windowed averages, and PWM duty/frequency from a trace.

/// How much history the live scope and averages keep (seconds).
pub const TRACE_WINDOW: f64 = 0.16;
/// Default spacing of recorded samples (seconds).
pub const SAMPLE_STEP: f64 = 50e-6;
/// Window used for the LED current readout (seconds).
pub const LED_AVG_WINDOW: f64 = 0.002;

/// Fixed-capacity ring of `(t, value)` samples for the live scope.
///
/// Memory is bounded by [`TraceBuf::capacity`], which is derived from the
/// time window and [`SAMPLE_STEP`] (plus a small slack). Samples older than
/// the window are dropped, and if the ring is full the oldest sample is
/// overwritten, so a long run cannot grow the buffer.
#[derive(Clone, Debug)]
pub struct TraceBuf {
    buf: Vec<(f64, f64)>,
    cap: usize,
    start: usize,
    len: usize,
    window: f64,
}

impl TraceBuf {
    /// A buffer that holds `window` seconds at `sample_step` spacing.
    pub fn with_window(window: f64, sample_step: f64) -> Self {
        let window = window.max(0.0);
        let step = if sample_step > 0.0 { sample_step } else { SAMPLE_STEP };
        let cap = ((window / step).ceil() as usize).saturating_add(8).max(16);
        Self {
            buf: vec![(0.0, 0.0); cap],
            cap,
            start: 0,
            len: 0,
            window,
        }
    }

    /// [`TRACE_WINDOW`] at [`SAMPLE_STEP`].
    pub fn default_scope() -> Self {
        Self::with_window(TRACE_WINDOW, SAMPLE_STEP)
    }

    /// Configured time window (seconds).
    #[allow(dead_code)]
    pub fn window(&self) -> f64 {
        self.window
    }

    /// Change the time window. Capacity is never increased, so memory stays flat.
    #[allow(dead_code)]
    pub fn set_window(&mut self, window: f64) {
        self.window = window.max(0.0);
        if let Some((t, _)) = self.last() {
            self.trim(t);
        }
    }

    /// Maximum number of samples this buffer will hold.
    #[allow(dead_code)]
    pub fn capacity(&self) -> usize {
        self.cap
    }

    /// Number of samples currently stored.
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    #[allow(dead_code)]
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
}
