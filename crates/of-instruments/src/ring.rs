//! Bounded sample rings and the fixed sample grid (moved from `of-ui` in E5a).
//!
//! [`TraceBuf`] keeps `(t, value)` samples for a time window with a hard
//! capacity cap; [`SampleClock`] decides when the next grid sample is due.
//! The scope constants live here because the ring sizes itself from them.

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
    pub fn len(&self) -> usize {
        self.len
    }

    #[allow(dead_code)]
    /// True when no samples are held.
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The newest sample.
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

    /// Samples oldest first.
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
    /// A grid starting at `start` with spacing `step` (seconds; non-positive means [`SAMPLE_STEP`]).
    pub fn new(start: f64, step: f64) -> Self {
        Self {
            next: start,
            step: if step > 0.0 { step } else { SAMPLE_STEP },
        }
    }

    /// Grid spacing (seconds).
    #[allow(dead_code)]
    pub fn step(&self) -> f64 {
        self.step
    }

    /// Time of the next grid sample (seconds).
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
