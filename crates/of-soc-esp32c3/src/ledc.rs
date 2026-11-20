//! LED PWM controller, LEDC (TRM chapter 32, base `0x6001_9000`).
//!
//! Four timers and six PWM channels (the C3's low-speed set), with the
//! register layout of the TRM and the `esp32c3` PAC.
//!
//! **Timers.** The global clock `LEDC_CONF.APB_CLK_SEL` (1 APB 80 MHz, 2
//! RC_FAST 17.5 MHz nominal, 3 XTAL 40 MHz, 0 none) is divided by the
//! fractional `CLK_DIV` (`A` in bits 21:12, `B`/256 in bits 11:4): of every
//! 256 counter ticks, `B` last `A + 1` clocks and the rest `A`, spread
//! evenly (tick `g` of a run ends after `floor((phase + g·(256A+B))/256)`
//! clocks). The 14-bit counter counts `0 .. 2^DUTY_RES - 1` and overflows
//! (`TIMERx_OVF` interrupt). `RST` holds it at 0, `PAUSE` freezes it; a
//! timer with no clock or `A = 0` does not count. `CLK_DIV` and
//! `DUTY_RES` take effect at the next overflow after `PARA_UP` (at once
//! while the timer is held in reset, and when it leaves reset).
//!
//! **Channels.** A channel follows its timer's counter: the output goes
//! high when the counter reaches `Hpoint` and low when it reaches `Lpoint =
//! Hpoint + DUTY[18:4]` (low wins if they are equal; an `Lpoint` past the
//! counter's range is never reached, so the output stays high). `Hpoint`,
//! `DUTY[18:4]`, `SIG_OUT_EN`, `TIMER_SEL` and `OVF_CNT_EN` are sampled at
//! the next overflow after the channel's `PARA_UP`; the fade fields
//! (`DUTY`, `CONF1`) take effect at the `PARA_UP` itself. Dither:
//! `DUTY[3:0]` of every 16 periods have `Lpoint` one tick later, spread
//! evenly (the TRM does not say which periods). With `SIG_OUT_EN` clear the
//! output is `IDLE_LV`. Fades: after `DUTY_START`, every `DUTY_CYCLE`
//! overflows (0 is taken as 1) the duty moves by `DUTY_SCALE` (integer
//! duty units, up with `DUTY_INC`), `DUTY_NUM` times, then
//! `DUTY_CHNG_END_CHn` is raised; `DUTY_R` reads the current duty.
//! `OVF_CNT_CHn` is raised every `OVF_NUM + 1` overflows with `OVF_CNT_EN`.
//!
//! **Outputs.** Each output change is a [`SignalChange`] stamped with the
//! exact time of the counter tick that caused it (not the time the hart
//! notices it), for the GPIO matrix signals `LEDC_LS_SIG_OUT0..5` (45..50).
//! [`Ledc::next_event`] is the next output change or the next overflow
//! that changes state, so the hart stops there. The interrupt is source 23
//! (`LEDC_INTR`): `INT_ST = INT_RAW & INT_ENA`.
//!
//! Not modelled: `TICK_SEL` (ignored), the `LEDC_CONF.CLK_EN` register
//! clock gate, the SYSTEM peripheral clock and reset bits, and any phase
//! relation between the timer clock and the CPU clock. `TIMERx_VALUE` reads
//! the counter as of the last sync point.

use of_core::SimTime;

/// Number of PWM channels.
pub const CHANNELS: usize = 6;
/// Number of timers.
pub const TIMERS: usize = 4;
/// GPIO matrix output signal of channel 0 (`LEDC_LS_SIG_OUT0`); channel n
/// is `SIGNAL_BASE + n`.
pub const SIGNAL_BASE: u8 = 45;
/// Largest counter width (bits).
pub const MAX_RES: u32 = 14;

/// Register offsets from [`crate::map::LEDC_BASE`].
pub mod reg {
    /// Distance between channel register sets.
    pub const CH_STRIDE: u32 = 0x14;
    /// `LEDC_CHn_CONF0` (n at `+ n * CH_STRIDE`).
    pub const CH_CONF0: u32 = 0x00;
    /// `LEDC_CHn_HPOINT`.
    pub const CH_HPOINT: u32 = 0x04;
    /// `LEDC_CHn_DUTY` (bits 18:4 integer, 3:0 dither).
    pub const CH_DUTY: u32 = 0x08;
    /// `LEDC_CHn_CONF1` (fade).
    pub const CH_CONF1: u32 = 0x0c;
    /// `LEDC_CHn_DUTY_R` (current duty, read-only).
    pub const CH_DUTY_R: u32 = 0x10;
    /// `LEDC_TIMERx_CONF` (x at `+ 8 * x`).
    pub const TIMER_CONF: u32 = 0xa0;
    /// `LEDC_TIMERx_VALUE` (counter, read-only).
    pub const TIMER_VALUE: u32 = 0xa4;
    /// Raw interrupt status: `TIMERx_OVF` bits 3:0, `DUTY_CHNG_END_CHn`
    /// bits 9:4, `OVF_CNT_CHn` bits 15:10.
    pub const INT_RAW: u32 = 0xc0;
    /// Masked interrupt status.
    pub const INT_ST: u32 = 0xc4;
    /// Interrupt enables.
    pub const INT_ENA: u32 = 0xc8;
    /// Write 1 to clear raw bits.
    pub const INT_CLR: u32 = 0xcc;
    /// Global configuration (`APB_CLK_SEL` bits 1:0, `CLK_EN` bit 31).
    pub const CONF: u32 = 0xd0;
}

/// `LEDC_CHn_CONF0` fields.
pub mod conf0 {
    /// `SIG_OUT_EN`.
    pub const SIG_OUT_EN: u32 = 1 << 2;
    /// `IDLE_LV`.
    pub const IDLE_LV: u32 = 1 << 3;
    /// `PARA_UP` (write-only trigger).
    pub const PARA_UP: u32 = 1 << 4;
    /// `OVF_NUM` shift (10 bits).
    pub const OVF_NUM_SHIFT: u32 = 5;
    /// `OVF_CNT_EN`.
    pub const OVF_CNT_EN: u32 = 1 << 15;
    /// `OVF_CNT_RESET` (write-only trigger).
    pub const OVF_CNT_RESET: u32 = 1 << 16;
}

/// `LEDC_CHn_CONF1` fields.
pub mod conf1 {
    /// `DUTY_START`.
    pub const DUTY_START: u32 = 1 << 31;
    /// `DUTY_INC` (reset value 1).
    pub const DUTY_INC: u32 = 1 << 30;
}

/// `LEDC_TIMERx_CONF` fields.
pub mod timer_conf {
    /// `CLK_DIV` shift (18 bits: integer part bits 21:12, fraction 11:4).
    pub const CLK_DIV_SHIFT: u32 = 4;
    /// `PAUSE`.
    pub const PAUSE: u32 = 1 << 22;
    /// `RST` (reset value 1).
    pub const RST: u32 = 1 << 23;
    /// `PARA_UP` (write-only trigger).
    pub const PARA_UP: u32 = 1 << 25;
}

/// An output change of channel `channel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SignalChange {
    /// When (the counter tick that caused it).
    pub at: SimTime,
    /// Channel number.
    pub channel: u8,
    /// New level.
    pub level: bool,
}

/// A timer counting: its time base and active parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Run {
    /// Time of tick 0 (counter = `count0`).
    epoch: SimTime,
    count0: u64,
    /// Fractional-divider phase at the epoch (0..256).
    phase: u64,
    hz: u64,
    /// `256 A + B`.
    div: u64,
    /// Counter range, `2^DUTY_RES`.
    n: u64,
}

impl Run {
    /// When tick `g` (counted from the epoch) happens.
    fn tick_time(&self, g: u64) -> SimTime {
        let clocks = (u128::from(self.phase) + u128::from(g) * u128::from(self.div)) / 256;
        let ps = clocks * 1_000_000_000_000 / u128::from(self.hz);
        SimTime(self.epoch.0.saturating_add(u64::try_from(ps).unwrap_or(u64::MAX)))
    }

    /// Ticks since the epoch at time `t` (the last tick at or before `t`).
    fn ticks_at(&self, t: SimTime) -> u64 {
        let Some(dt) = t.0.checked_sub(self.epoch.0) else {
            return 0;
        };
        let clocks = ((u128::from(dt) + 1) * u128::from(self.hz) - 1) / 1_000_000_000_000;
        let g = (256 * (clocks + 1) - 1 - u128::from(self.phase)) / u128::from(self.div);
        u64::try_from(g).unwrap_or(u64::MAX)
    }

    /// Tick (from the epoch) of the next overflow.
    fn ovf_tick(&self) -> u64 {
        self.n.saturating_sub(self.count0).max(1)
    }

    /// The counter value at `t`.
    fn count_at(&self, t: SimTime) -> u64 {
        (self.count0 + self.ticks_at(t)) % self.n
    }

    /// The same run continued from tick `g`, with the counter at `count`.
    fn rebase(&self, g: u64, count: u64) -> Self {
        Self {
            epoch: self.tick_time(g),
            count0: count,
            phase: (u128::from(self.phase) + u128::from(g) * u128::from(self.div)).rem_euclid(256) as u64,
            ..*self
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Timer {
    /// `TIMERx_CONF` as written (without `PARA_UP`).
    conf: u32,
    /// Active `256 A + B` and `DUTY_RES`.
    div: u64,
    res: u32,
    pending: bool,
    run: Option<Run>,
    /// Counter value while stopped.
    count: u64,
}

impl Timer {
    fn new() -> Self {
        Self {
            conf: timer_conf::RST,
            div: 0,
            res: 0,
            pending: false,
            run: None,
            count: 0,
        }
    }

    fn latch(&mut self) {
        self.div = u64::from((self.conf >> timer_conf::CLK_DIV_SHIFT) & 0x3_ffff);
        self.res = (self.conf & 0xf).min(MAX_RES);
        self.pending = false;
    }

    fn n(&self) -> u64 {
        1 << self.res
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Fade {
    inc: bool,
    num: u32,
    cycle: u32,
    scale: u32,
    steps: u32,
    cycles: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Channel {
    conf0: u32,
    hpoint: u32,
    duty: u32,
    conf1: u32,
    /// Sampled at an overflow: timer, output enable, `Hpoint`, `OVF_CNT_EN`.
    timer: usize,
    out_en: bool,
    hpoint_l: u64,
    ovf_cnt_en: bool,
    pending: bool,
    /// Current duty (`DUTY_R`, 19 bits).
    duty_cur: u32,
    fade: Option<Fade>,
    /// Period number within the 16-period dither cycle.
    dither_k: u32,
    ovf_cnt: u32,
    level: bool,
    /// Output changes due in the current period, in time order.
    events: Vec<(SimTime, bool)>,
}

impl Channel {
    fn new() -> Self {
        Self {
            conf0: 0,
            hpoint: 0,
            duty: 0,
            conf1: conf1::DUTY_INC,
            timer: 0,
            out_en: false,
            hpoint_l: 0,
            ovf_cnt_en: false,
            pending: false,
            duty_cur: 0,
            fade: None,
            dither_k: 0,
            ovf_cnt: 0,
            level: false,
            events: Vec::new(),
        }
    }

    /// Timer whose next overflow samples this channel.
    fn next_timer(&self) -> usize {
        if self.pending {
            (self.conf0 & 3) as usize
        } else {
            self.timer
        }
    }

    /// Whether an overflow of its timer changes anything for this channel.
    fn busy(&self) -> bool {
        self.out_en || self.pending || self.fade.is_some() || self.ovf_cnt_en
    }
}

/// The LED PWM controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ledc {
    conf: u32,
    timers: [Timer; TIMERS],
    ch: [Channel; CHANNELS],
    int_raw: u32,
    int_ena: u32,
    now: SimTime,
    changes: Vec<SignalChange>,
}

impl Default for Ledc {
    fn default() -> Self {
        Self::new()
    }
}

/// Clock frequency for `LEDC_CONF.APB_CLK_SEL`.
fn clock_hz(conf: u32) -> Option<u64> {
    match conf & 3 {
        1 => Some(80_000_000),
        2 => Some(17_500_000),
        3 => Some(40_000_000),
        _ => None,
    }
}

impl Ledc {
    /// Reset state: timers held in reset, channels idle low.
    pub fn new() -> Self {
        Self {
            conf: 0,
            timers: std::array::from_fn(|_| Timer::new()),
            ch: std::array::from_fn(|_| Channel::new()),
            int_raw: 0,
            int_ena: 0,
            now: SimTime::ZERO,
            changes: Vec::new(),
        }
    }

    /// Whether `off` is an LEDC register (all are applied at the hart's
    /// time).
    pub fn is_reg(off: u32) -> bool {
        off < 0x100
    }

    /// Interrupt output (source 23).
    pub fn irq(&self) -> bool {
        self.int_raw & self.int_ena != 0
    }

    /// Current output level of channel `n`.
    pub fn level(&self, n: usize) -> bool {
        self.ch.get(n).is_some_and(|c| c.level)
    }

    /// Output changes since the last call, in time order.
    pub fn take_changes(&mut self) -> Vec<SignalChange> {
        std::mem::take(&mut self.changes)
    }

    /// Whether an overflow of timer `x` changes any state.
    fn ovf_needed(&self, x: usize) -> bool {
        self.int_raw & (1 << x) == 0
            || self.timers[x].pending
            || self.ch.iter().any(|c| c.next_timer() == x && c.busy())
    }

    fn next_ovf(&self, x: usize) -> Option<SimTime> {
        let r = self.timers[x].run?;
        Some(r.tick_time(r.ovf_tick()))
    }

    /// Earliest time at which the controller needs [`Self::advance`]: the
    /// next output change or the next overflow that changes state.
    pub fn next_event(&self) -> Option<SimTime> {
        let ovf = (0..TIMERS)
            .filter(|&x| self.ovf_needed(x))
            .filter_map(|x| self.next_ovf(x));
        let edges = self.ch.iter().filter_map(|c| c.events.first().map(|e| e.0));
        ovf.chain(edges).min()
    }

    /// Run to `t`: overflows, fades and output changes up to and including
    /// `t`.
    pub fn advance(&mut self, t: SimTime) {
        loop {
            let ovf = (0..TIMERS).filter_map(|x| self.next_ovf(x).map(|at| (at, x))).min();
            let edge = (0..CHANNELS)
                .filter_map(|n| self.ch[n].events.first().map(|e| (e.0, n)))
                .min();
            match (ovf, edge) {
                (Some((at, x)), e) if at <= t && e.is_none_or(|(et, _)| at <= et) => {
                    if self.ovf_needed(x) {
                        self.overflow(x, at);
                    } else {
                        self.skip_overflows(x, t);
                    }
                }
                (_, Some((at, n))) if at <= t => {
                    let (at, level) = self.ch[n].events.remove(0);
                    self.set_level(n, at, level);
                }
                _ => break,
            }
        }
        self.now = self.now.max(t);
    }

    fn set_level(&mut self, n: usize, at: SimTime, level: bool) {
        if self.ch[n].level != level {
            self.ch[n].level = level;
            self.changes.push(SignalChange {
                at,
                channel: n as u8,
                level,
            });
        }
    }

    /// Roll timer `x` past every overflow up to `t` that changes nothing.
    fn skip_overflows(&mut self, x: usize, t: SimTime) {
        let Some(r) = self.timers[x].run else { return };
        let total = r.count0 + r.ticks_at(t);
        let periods = total / r.n;
        if periods > 0 {
            self.timers[x].run = Some(r.rebase(periods * r.n - r.count0, 0));
        }
    }

    /// Timer `x` overflows at `at`.
    fn overflow(&mut self, x: usize, at: SimTime) {
        self.int_raw |= 1 << x;
        let Some(r) = self.timers[x].run else { return };
        let mut next = r.rebase(r.ovf_tick(), 0);
        let tm = &mut self.timers[x];
        if tm.pending {
            tm.latch();
            next.div = tm.div;
            next.n = tm.n();
        }
        tm.run = (next.div >= 256).then_some(next);
        for n in 0..CHANNELS {
            if self.ch[n].next_timer() != x {
                continue;
            }
            let c = &mut self.ch[n];
            // Overflows count from the one after OVF_CNT_EN is sampled.
            let counting = c.ovf_cnt_en;
            if c.pending {
                c.pending = false;
                c.timer = x;
                c.out_en = c.conf0 & conf0::SIG_OUT_EN != 0;
                c.hpoint_l = u64::from(c.hpoint);
                c.ovf_cnt_en = c.conf0 & conf0::OVF_CNT_EN != 0;
            }
            if counting && c.ovf_cnt_en {
                c.ovf_cnt += 1;
                if c.ovf_cnt > (c.conf0 >> conf0::OVF_NUM_SHIFT) & 0x3ff {
                    c.ovf_cnt = 0;
                    self.int_raw |= 1 << (10 + n);
                }
            }
            if let Some(mut f) = c.fade {
                let mut done = f.num == 0;
                if !done {
                    f.cycles += 1;
                    if f.cycles >= f.cycle.max(1) {
                        f.cycles = 0;
                        let step = f.scale << 4;
                        c.duty_cur = if f.inc {
                            c.duty_cur.saturating_add(step).min(0x7_ffff)
                        } else {
                            c.duty_cur.saturating_sub(step)
                        };
                        f.steps += 1;
                        done = f.steps >= f.num;
                    }
                }
                c.fade = (!done).then_some(f);
                if done {
                    self.int_raw |= 1 << (4 + n);
                }
            }
            let c = &mut self.ch[n];
            let k = c.dither_k;
            c.dither_k = (k + 1) % 16;
            if !c.out_en {
                let idle = c.conf0 & conf0::IDLE_LV != 0;
                c.events.clear();
                self.set_level(n, at, idle);
                continue;
            }
            let frac = c.duty_cur & 0xf;
            let extra = u64::from((k + 1) * frac / 16 - k * frac / 16);
            self.plan(n, true, extra);
        }
    }

    /// Plan channel `n`'s output changes for the rest of its timer's
    /// current period (`at_start`: the counter is at the start of the
    /// period, so a point equal to it fires now).
    fn plan(&mut self, n: usize, at_start: bool, extra: u64) {
        let c = &self.ch[n];
        let Some(r) = self.timers[c.timer].run else {
            self.ch[n].events.clear();
            return;
        };
        let h = c.hpoint_l;
        let l = h + u64::from(c.duty_cur >> 4) + extra;
        let due = |v: u64| v < r.n && (v > r.count0 || (at_start && v == r.count0));
        let mut ev = Vec::with_capacity(2);
        if due(h) && h != l {
            ev.push((h, true));
        }
        if due(l) {
            ev.push((l, false));
        }
        ev.sort_by_key(|e| e.0);
        self.ch[n].events = ev.into_iter().map(|(v, lv)| (r.tick_time(v - r.count0), lv)).collect();
    }

    /// Start, stop or retime timer `x` at `t` after a change of its
    /// configuration or of the clock.
    fn retime(&mut self, x: usize, t: SimTime) {
        let hz = clock_hz(self.conf);
        let tm = &mut self.timers[x];
        let was = tm.run;
        let count = was.map_or(tm.count, |r| r.count_at(t));
        let rst = tm.conf & timer_conf::RST != 0;
        let count = if rst { 0 } else { count };
        let runs = !rst && tm.conf & timer_conf::PAUSE == 0 && tm.div >= 256;
        tm.count = count;
        tm.run = match (runs, hz) {
            (true, Some(hz)) => match was {
                Some(r) if r.hz == hz => Some(r),
                _ => Some(Run {
                    epoch: t,
                    count0: count,
                    phase: 0,
                    hz,
                    div: tm.div,
                    n: tm.n(),
                }),
            },
            _ => None,
        };
        if tm.run != was {
            let started = was.is_none() && count == 0;
            for n in 0..CHANNELS {
                if self.ch[n].timer == x && self.ch[n].out_en {
                    let frac = self.ch[n].duty_cur & 0xf;
                    let k = self.ch[n].dither_k;
                    let extra = u64::from((k + 1) * frac / 16 - k * frac / 16);
                    self.plan(n, started, extra);
                }
            }
        }
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        if off < reg::CH_STRIDE * CHANNELS as u32 {
            let c = &self.ch[(off / reg::CH_STRIDE) as usize];
            return Some(match off % reg::CH_STRIDE {
                reg::CH_CONF0 => c.conf0,
                reg::CH_HPOINT => c.hpoint,
                reg::CH_DUTY => c.duty,
                reg::CH_CONF1 => c.conf1,
                _ => c.duty_cur,
            });
        }
        match off {
            0xa0..=0xbc => {
                let tm = &self.timers[((off - reg::TIMER_CONF) / 8) as usize];
                Some(if off.is_multiple_of(8) {
                    tm.conf
                } else {
                    tm.run.map_or(tm.count, |r| r.count_at(self.now)) as u32
                })
            }
            reg::INT_RAW => Some(self.int_raw),
            reg::INT_ST => Some(self.int_raw & self.int_ena),
            reg::INT_ENA => Some(self.int_ena),
            reg::INT_CLR => Some(0),
            reg::CONF => Some(self.conf),
            _ => None,
        }
    }

    /// Write a register at time `t` (call [`Self::advance`] to `t` first).
    pub fn write(&mut self, off: u32, v: u32, t: SimTime) -> Option<()> {
        if off < reg::CH_STRIDE * CHANNELS as u32 {
            let n = (off / reg::CH_STRIDE) as usize;
            let c = &mut self.ch[n];
            match off % reg::CH_STRIDE {
                reg::CH_CONF0 => {
                    c.conf0 = v & 0xffff & !conf0::PARA_UP;
                    if v & conf0::OVF_CNT_RESET != 0 {
                        c.ovf_cnt = 0;
                    }
                    if v & conf0::PARA_UP != 0 {
                        c.pending = true;
                        c.duty_cur = c.duty;
                        c.fade = (c.conf1 & conf1::DUTY_START != 0).then_some(Fade {
                            inc: c.conf1 & conf1::DUTY_INC != 0,
                            num: (c.conf1 >> 20) & 0x3ff,
                            cycle: (c.conf1 >> 10) & 0x3ff,
                            scale: c.conf1 & 0x3ff,
                            steps: 0,
                            cycles: 0,
                        });
                    }
                    if !c.out_en {
                        let idle = c.conf0 & conf0::IDLE_LV != 0;
                        self.set_level(n, t, idle);
                    }
                }
                reg::CH_HPOINT => c.hpoint = v & 0x3fff,
                reg::CH_DUTY => c.duty = v & 0x7_ffff,
                reg::CH_CONF1 => c.conf1 = v,
                _ => {}
            }
            return Some(());
        }
        match off {
            0xa0..=0xbc if off.is_multiple_of(8) => {
                let x = ((off - reg::TIMER_CONF) / 8) as usize;
                let tm = &mut self.timers[x];
                let was_reset = tm.conf & timer_conf::RST != 0;
                tm.conf = v & 0x01ff_ffff & !timer_conf::PARA_UP;
                if v & timer_conf::PARA_UP != 0 {
                    tm.pending = true;
                }
                // Held in reset, or leaving it: the settings apply at once.
                if was_reset || v & timer_conf::RST != 0 {
                    tm.latch();
                }
                self.retime(x, t);
            }
            0xa0..=0xbc => {}
            reg::INT_RAW | reg::INT_ST => {}
            reg::INT_ENA => self.int_ena = v & 0xffff,
            reg::INT_CLR => self.int_raw &= !v,
            reg::CONF => {
                self.conf = v & 0x8000_0003;
                for x in 0..TIMERS {
                    self.retime(x, t);
                }
            }
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONF: u32 = reg::CONF;

    fn ns(t: u64) -> SimTime {
        SimTime::from_ns(t)
    }

    /// Write at `t`, after advancing there.
    fn w(l: &mut Ledc, off: u32, v: u32, t: SimTime) {
        l.advance(t);
        l.write(off, v, t).unwrap();
    }

    /// esp-hal's sequence: APB clock, timer 0 at `div` (256 A + B) and
    /// `res` bits, channel `n` on timer 0 at integer `duty`.
    fn setup(l: &mut Ledc, n: u32, div: u32, res: u32, duty: u32, t: SimTime) {
        w(l, CONF, 1, t);
        w(l, reg::TIMER_CONF, timer_conf::RST | timer_conf::PARA_UP, t);
        w(l, reg::TIMER_CONF, (div << 4) | res, t);
        w(l, reg::TIMER_CONF, (div << 4) | res | timer_conf::PARA_UP, t);
        let ch = n * reg::CH_STRIDE;
        w(l, ch + reg::CH_DUTY, duty << 4, t);
        w(
            l,
            ch + reg::CH_CONF1,
            conf1::DUTY_START | conf1::DUTY_INC | (1 << 20) | (1 << 10),
            t,
        );
        w(l, ch + reg::CH_HPOINT, 0, t);
        w(l, ch + reg::CH_CONF0, conf0::SIG_OUT_EN, t);
        w(l, ch + reg::CH_CONF0, conf0::SIG_OUT_EN | conf0::PARA_UP, t);
    }

    /// Run to `end`, collecting output changes.
    fn run(l: &mut Ledc, end: SimTime) -> Vec<SignalChange> {
        let mut out = Vec::new();
        while let Some(t) = l.next_event().filter(|&t| t <= end) {
            l.advance(t);
            out.extend(l.take_changes());
        }
        l.advance(end);
        out.extend(l.take_changes());
        out
    }

    #[test]
    fn pwm_period_and_duty_follow_the_registers() {
        let mut l = Ledc::new();
        // 80 MHz / (5 * 2^8) = 62.5 kHz, 16 µs period; duty 64/256 = 4 µs.
        let t0 = ns(1_000);
        setup(&mut l, 0, 5 << 8, 8, 64, t0);
        let out = run(&mut l, t0.saturating_add(SimTime::from_us(100)));
        // The first overflow (16 µs after the timer starts) samples the
        // channel; from then on high at every overflow, low 4 µs later.
        let first = t0.saturating_add(SimTime::from_us(16));
        assert_eq!(
            out[0],
            SignalChange {
                at: first,
                channel: 0,
                level: true
            }
        );
        for (i, pair) in out.chunks(2).enumerate() {
            let start = first.saturating_add(SimTime::from_us(16 * i as u64));
            assert_eq!(pair[0].at, start, "{i}");
            assert!(pair[0].level);
            if let Some(low) = pair.get(1) {
                assert_eq!(low.at, start.saturating_add(SimTime::from_us(4)));
                assert!(!low.level);
            }
        }
        assert_eq!(out.len(), 12, "{out:?}");
        assert_eq!(l.read(reg::INT_RAW).map(|v| v & 1), Some(1), "TIMER0_OVF");
        assert_eq!(l.read(reg::CH_DUTY_R), Some(64 << 4));
        assert!(
            l.read(reg::INT_RAW).unwrap() & (1 << 4) != 0,
            "the one-step 'fade' ended"
        );
    }

    #[test]
    fn zero_and_full_duty_are_constant_levels() {
        for (duty, level) in [(0, None), (256, Some(true))] {
            let mut l = Ledc::new();
            setup(&mut l, 2, 1 << 8, 8, duty, SimTime::ZERO);
            let out = run(&mut l, SimTime::from_us(50));
            assert_eq!(out.first().map(|c| c.level), level, "duty {duty}: {out:?}");
            assert!(out.len() <= 1);
            assert_eq!(out.first().map_or(2, |c| c.channel), 2);
        }
    }

    #[test]
    fn fractional_divider_spreads_the_long_ticks() {
        // A = 3, B = 128: ticks alternate 3 and 4 clocks; 2^4 ticks = 56 clocks.
        let mut l = Ledc::new();
        setup(&mut l, 0, (3 << 8) | 128, 4, 8, SimTime::ZERO);
        let out = run(&mut l, SimTime::from_us(20));
        let rises: Vec<u64> = out.iter().filter(|c| c.level).map(|c| c.at.0).collect();
        for p in rises.windows(2) {
            assert_eq!(p[1] - p[0], 56 * 12_500, "{rises:?}");
        }
        // 8 ticks of 3.5 clocks: 28 clocks high.
        let high = out.iter().position(|c| c.level).unwrap();
        assert_eq!(out[high + 1].at.0 - out[high].at.0, 28 * 12_500);
    }

    #[test]
    fn dither_lengthens_some_periods_by_one_tick() {
        let mut l = Ledc::new();
        setup(&mut l, 0, 1 << 8, 4, 0, SimTime::ZERO);
        // Duty 5 + 4/16: 4 of every 16 periods are 6 ticks high.
        w(&mut l, reg::CH_DUTY, (5 << 4) | 4, SimTime::ZERO);
        w(&mut l, reg::CH_CONF0, conf0::SIG_OUT_EN | conf0::PARA_UP, SimTime::ZERO);
        let tick = 12_500;
        let out = run(&mut l, SimTime(tick * 16 * 40));
        let highs: Vec<u64> = out
            .windows(2)
            .filter(|p| p[0].level && !p[1].level)
            .map(|p| (p[1].at.0 - p[0].at.0) / tick)
            .collect();
        let window = &highs[highs.len() - 16..];
        assert_eq!(window.iter().filter(|&&h| h == 6).count(), 4, "{highs:?}");
        assert_eq!(window.iter().filter(|&&h| h == 5).count(), 12);
    }

    #[test]
    fn fade_steps_the_duty_and_raises_the_end_interrupt() {
        let mut l = Ledc::new();
        setup(&mut l, 1, 1 << 8, 8, 10, SimTime::ZERO);
        let t = SimTime::from_us(10);
        let ch = reg::CH_STRIDE;
        // From 10 up by 2 every 3 overflows, 5 times: ends at 20.
        w(&mut l, reg::INT_CLR, 1 << 5, t);
        w(&mut l, ch + reg::CH_DUTY, 10 << 4, t);
        w(
            &mut l,
            ch + reg::CH_CONF1,
            conf1::DUTY_START | conf1::DUTY_INC | (5 << 20) | (3 << 10) | 2,
            t,
        );
        w(&mut l, ch + reg::CH_CONF0, conf0::SIG_OUT_EN | conf0::PARA_UP, t);
        assert_eq!(l.read(ch + reg::CH_DUTY_R), Some(10 << 4), "fade fields apply at once");
        let period = 256 * 12_500;
        run(&mut l, SimTime(t.0 + 14 * period));
        assert_eq!(l.read(reg::INT_RAW).unwrap() & (1 << 5), 0, "still fading");
        run(&mut l, SimTime(t.0 + 16 * period));
        assert_eq!(l.read(ch + reg::CH_DUTY_R), Some(20 << 4));
        assert_ne!(l.read(reg::INT_RAW).unwrap() & (1 << 5), 0);
        l.write(reg::INT_ENA, 1 << 5, SimTime(t.0 + 16 * period)).unwrap();
        assert!(l.irq());
    }

    #[test]
    fn pause_freezes_the_counter_and_reset_holds_it() {
        let mut l = Ledc::new();
        setup(&mut l, 0, 1 << 8, 8, 128, SimTime::ZERO);
        let t = SimTime(100 * 12_500);
        l.advance(t);
        assert_eq!(l.read(reg::TIMER_VALUE), Some(100));
        let conf = l.read(reg::TIMER_CONF).unwrap();
        w(&mut l, reg::TIMER_CONF, conf | timer_conf::PAUSE, t);
        assert_eq!(l.next_event(), None, "nothing happens while paused");
        let t2 = SimTime(t.0 * 3);
        w(&mut l, reg::TIMER_CONF, conf, t2);
        l.advance(SimTime(t2.0 + 10 * 12_500));
        assert_eq!(l.read(reg::TIMER_VALUE), Some(110));
        w(
            &mut l,
            reg::TIMER_CONF,
            conf | timer_conf::RST,
            SimTime(t2.0 + 20 * 12_500),
        );
        assert_eq!(l.read(reg::TIMER_VALUE), Some(0));
        assert_eq!(l.next_event(), None);
    }

    #[test]
    fn idle_level_and_ovf_count_interrupt() {
        let mut l = Ledc::new();
        w(&mut l, reg::CH_CONF0, conf0::IDLE_LV, SimTime::ZERO);
        assert!(l.level(0));
        assert_eq!(l.take_changes().len(), 1);
        // Count 3 overflows of timer 0 on channel 0.
        setup(&mut l, 1, 1 << 8, 4, 0, SimTime::ZERO);
        let ch = reg::CH_STRIDE;
        w(
            &mut l,
            ch + reg::CH_CONF0,
            conf0::OVF_CNT_EN | (2 << 5) | conf0::PARA_UP,
            SimTime::ZERO,
        );
        let ovf = 16 * 12_500;
        l.advance(SimTime(3 * ovf));
        assert_eq!(
            l.read(reg::INT_RAW).unwrap() & (1 << 11),
            0,
            "latched at the first overflow"
        );
        l.advance(SimTime(4 * ovf));
        assert_ne!(l.read(reg::INT_RAW).unwrap() & (1 << 11), 0);
        assert!(l.read(CONF).is_some());
    }

    #[test]
    fn unneeded_overflows_are_skipped_but_the_counter_runs() {
        let mut l = Ledc::new();
        w(&mut l, CONF, 1, SimTime::ZERO);
        w(&mut l, reg::TIMER_CONF, (1 << 12) | 4, SimTime::ZERO);
        let ovf = 16 * 12_500;
        assert_eq!(l.next_event(), Some(SimTime(ovf)), "first overflow sets TIMER0_OVF");
        l.advance(SimTime(ovf));
        assert_eq!(l.next_event(), None, "nothing more to do");
        l.advance(SimTime(1000 * ovf + 3 * 12_500));
        assert_eq!(l.read(reg::TIMER_VALUE), Some(3));
        w(&mut l, reg::INT_CLR, 1, SimTime(1000 * ovf + 3 * 12_500));
        assert_eq!(l.next_event(), Some(SimTime(1001 * ovf)));
    }
}
