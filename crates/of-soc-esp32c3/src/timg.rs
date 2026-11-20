//! Timer groups TIMG0 and TIMG1 (TRM chapters 11 and 12): each has one
//! 54-bit general-purpose timer (T0) and a main system watchdog (MWDT).
//!
//! **Timer.** Counts ticks of APB (80 MHz) or XTAL (40 MHz, `USE_XTAL`)
//! divided by `DIVIDER` (0 means 65536, 1 means 2) while `EN` is set, up or
//! down (`INCREASE`). `T0UPDATE` latches the count into `T0LO/HI`;
//! `T0LOAD` copies `T0LOADLO/HI` into the counter. With `ALARM_EN` set the
//! alarm fires when an up-counter is at or above the alarm value (a
//! down-counter at or below), so an alarm already passed fires at once. An
//! alarm sets `INT_RAW.T0`, clears `ALARM_EN` and, with `AUTORELOAD`, reloads
//! the counter. Changing the clock or divider, `DIVCNT_RST` or enabling the
//! timer restarts the prescaler phase.
//!
//! **MWDT.** Four stages (see [`crate::watchdog`]) timed in ticks of
//! 12.5 ns x `WDT_CLK_PRESCALE` (25 ns with `WDT_USE_XTAL`); stage actions
//! 0 off, 1 interrupt (`INT_RAW.WDT`), 2 CPU reset, 3 system reset. Writes to
//! the configuration and feed registers are ignored unless `WDTWPROTECT`
//! holds the key `0x50D8_3AA1` (its reset value). New settings apply when
//! written; `WDT_CONF_UPDATE_EN` is accepted but not needed. In flash boot
//! the ROM leaves TIMG0's MWDT armed through `WDT_FLASHBOOT_MOD_EN`: when
//! armed ([`Timg::arm_boot_protection`]) and that bit is set, stage 0
//! resets the system after `WDT_STG0_HOLD` ticks even with `WDT_EN` clear.
//!
//! **RTC clock calibration** (`RTCCALICFG`, `RTCCALICFG1`, `RTCCALICFG2`):
//! counts XTAL (40 MHz) cycles over `RTC_CALI_MAX` cycles of the clock
//! chosen by `RTC_CALI_CLK_SEL` (0 RC_SLOW, 1 RC_FAST / 256, 2 XTAL32K). A
//! rising `RTC_CALI_START` starts a one-shot count; `RTC_CALI_RDY` sets when
//! it ends, with the count in `RTC_CALI_VALUE`. `RTC_CALI_START_CYCLING`
//! (set at reset) repeats the count, so a result is ready one count after
//! cycling starts. A count longer than `RTC_CALI_TIMEOUT_THRES` XTAL cycles
//! sets `RTC_CALI_TIMEOUT` at that point instead. The oscillators run at
//! their nominal frequencies ([`RC_SLOW_HZ`], [`RC_FAST_DIV_HZ`]); no
//! 32 kHz crystal is fitted, so XTAL32K counts always time out.
//!
//! `INT_ST = INT_RAW & INT_ENA` drives the level sources `TGn_T0` and
//! `TGn_WDT`. Writes are applied with the time of the instruction that made
//! them; call [`Timg::advance`] to that time first.

use of_core::SimTime;

use crate::watchdog::{Action, ResetKind, Stages, Timing};

/// APB clock period (80 MHz).
pub const APB_PS: u64 = 12_500;
/// XTAL clock period (40 MHz).
pub const XTAL_PS: u64 = 25_000;
/// The MWDT write-protection key.
pub const WDT_WKEY: u32 = 0x50d8_3aa1;
const MASK54: u64 = (1 << 54) - 1;
/// Nominal RC_SLOW frequency (136 kHz).
pub const RC_SLOW_HZ: u64 = 136_000;
/// Nominal RC_FAST (17.5 MHz) divided by 256, as the calibration sees it.
pub const RC_FAST_DIV_HZ: u64 = 17_500_000 / 256;
/// XTAL frequency.
const XTAL_HZ: u64 = 40_000_000;

/// Register offsets from a timer group's base.
pub mod reg {
    /// Timer 0 configuration (see [`super::t0`]).
    pub const T0CONFIG: u32 = 0x00;
    /// Latched count, bits 31:0.
    pub const T0LO: u32 = 0x04;
    /// Latched count, bits 53:32.
    pub const T0HI: u32 = 0x08;
    /// Write to latch the count.
    pub const T0UPDATE: u32 = 0x0c;
    /// Alarm value, bits 31:0.
    pub const T0ALARMLO: u32 = 0x10;
    /// Alarm value, bits 53:32.
    pub const T0ALARMHI: u32 = 0x14;
    /// Reload value, bits 31:0.
    pub const T0LOADLO: u32 = 0x18;
    /// Reload value, bits 53:32.
    pub const T0LOADHI: u32 = 0x1c;
    /// Write to load the counter from `T0LOADLO/HI`.
    pub const T0LOAD: u32 = 0x20;
    /// MWDT configuration (see [`super::wdt`]).
    pub const WDTCONFIG0: u32 = 0x48;
    /// MWDT prescaler, bits 31:16.
    pub const WDTCONFIG1: u32 = 0x4c;
    /// Stage 0 timeout in MWDT ticks (stages 1 to 3 follow at +4).
    pub const WDTCONFIG2: u32 = 0x50;
    /// Stage 3 timeout.
    pub const WDTCONFIG5: u32 = 0x5c;
    /// Write anything to feed the MWDT.
    pub const WDTFEED: u32 = 0x60;
    /// Write-protection key.
    pub const WDTWPROTECT: u32 = 0x64;
    /// Interrupt enable: bit 0 timer, bit 1 watchdog.
    pub const INT_ENA: u32 = 0x70;
    /// Raw interrupt status (write 1 to clear).
    pub const INT_RAW: u32 = 0x74;
    /// Masked interrupt status.
    pub const INT_ST: u32 = 0x78;
    /// Interrupt clear.
    pub const INT_CLR: u32 = 0x7c;
    /// RTC calibration configuration (see [`super::cali`]).
    pub const RTCCALICFG: u32 = 0x68;
    /// RTC calibration result: `RTC_CALI_VALUE` bits 31:7,
    /// `CYCLING_DATA_VLD` bit 0.
    pub const RTCCALICFG1: u32 = 0x6c;
    /// RTC calibration timeout: `TIMEOUT_THRES` bits 31:7,
    /// `TIMEOUT_RST_CNT` bits 6:3, `TIMEOUT` bit 0.
    pub const RTCCALICFG2: u32 = 0x80;
    /// Version register.
    pub const NTIMERS_DATE: u32 = 0xf8;
    /// Register clock control.
    pub const REGCLK: u32 = 0xfc;
}

/// `T0CONFIG` bits.
pub mod t0 {
    /// Counter enable.
    pub const EN: u32 = 1 << 31;
    /// Count up (else down).
    pub const INCREASE: u32 = 1 << 30;
    /// Reload on alarm.
    pub const AUTORELOAD: u32 = 1 << 29;
    /// Prescaler field shift (bits 28:13).
    pub const DIVIDER_SHIFT: u32 = 13;
    /// Restart the prescaler (write-only).
    pub const DIVCNT_RST: u32 = 1 << 12;
    /// Alarm enable (cleared by hardware when the alarm fires).
    pub const ALARM_EN: u32 = 1 << 10;
    /// Clock from XTAL instead of APB.
    pub const USE_XTAL: u32 = 1 << 9;
    /// Reset value: divider 1 (= 2), auto-reload, counting up, stopped.
    pub const RESET: u32 = INCREASE | AUTORELOAD | (1 << DIVIDER_SHIFT);
}

/// `WDTCONFIG0` bits.
pub mod wdt {
    /// MWDT enable.
    pub const EN: u32 = 1 << 31;
    /// Stage 0 action shift (2 bits); stage n is at `STG0_SHIFT - 2n`.
    pub const STG0_SHIFT: u32 = 29;
    /// Apply new configuration (write-only).
    pub const CONF_UPDATE_EN: u32 = 1 << 22;
    /// Clock from XTAL instead of APB.
    pub const USE_XTAL: u32 = 1 << 21;
    /// Flash boot protection.
    pub const FLASHBOOT_MOD_EN: u32 = 1 << 14;
    /// Reset value: flash boot protection, reset lengths 1.
    pub const RESET: u32 = FLASHBOOT_MOD_EN | (1 << 15) | (1 << 18);
}

/// `RTCCALICFG` bits.
pub mod cali {
    /// Repeat counts continuously.
    pub const START_CYCLING: u32 = 1 << 12;
    /// Clock select shift (bits 14:13).
    pub const CLK_SEL_SHIFT: u32 = 13;
    /// Count finished (read-only).
    pub const RDY: u32 = 1 << 15;
    /// Slow-clock cycles to count, shift (bits 30:16).
    pub const MAX_SHIFT: u32 = 16;
    /// Start a one-shot count.
    pub const START: u32 = 1 << 31;
    /// Reset value: cycling, RC_FAST / 256, one cycle.
    pub const RESET: u32 = 0x0001_3000;
    /// `RTCCALICFG2` reset value.
    pub const CFG2_RESET: u32 = 0xffff_ff98;
}

/// The RTC calibration counter.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Calibration {
    cfg: u32,
    cfg2: u32,
    /// When the running count ends, if one is running.
    ends: Option<SimTime>,
    /// Whether that end is a timeout rather than a result.
    times_out: bool,
    rdy: bool,
    timeout: bool,
    value: u32,
}

impl Calibration {
    fn new() -> Self {
        let mut c = Self {
            cfg: cali::RESET,
            cfg2: cali::CFG2_RESET,
            ends: None,
            times_out: false,
            rdy: false,
            timeout: false,
            value: 0,
        };
        // Cycling from reset.
        c.start(SimTime::ZERO);
        c
    }

    /// Slow clock frequency for the current `CLK_SEL`, if it runs.
    fn slow_hz(&self) -> Option<u64> {
        match (self.cfg >> cali::CLK_SEL_SHIFT) & 3 {
            0 => Some(RC_SLOW_HZ),
            1 => Some(RC_FAST_DIV_HZ),
            _ => None,
        }
    }

    /// Start a count at `t`.
    fn start(&mut self, t: SimTime) {
        let max = u64::from((self.cfg >> cali::MAX_SHIFT) & 0x7fff);
        let thres = u64::from(self.cfg2 >> 7);
        let xtal_cycles = self.slow_hz().map(|hz| max * XTAL_HZ / hz);
        self.rdy = false;
        self.timeout = false;
        match xtal_cycles {
            Some(n) if n <= thres => {
                self.value = n as u32;
                self.times_out = false;
                self.ends = Some(SimTime(t.0 + n * XTAL_PS));
            }
            _ => {
                self.times_out = true;
                self.ends = Some(SimTime(t.0 + thres * XTAL_PS));
            }
        }
    }

    fn running(&self) -> bool {
        self.cfg & (cali::START | cali::START_CYCLING) != 0
    }

    /// End of a one-shot count. (A cycling count's result is only looked
    /// at when polled, so it needs no exact stop.)
    fn next_event(&self) -> Option<SimTime> {
        self.ends.filter(|_| self.cfg & cali::START != 0)
    }

    fn advance(&mut self, t: SimTime) {
        if let Some(end) = self.ends.filter(|_| self.running())
            && t >= end
        {
            self.ends = None;
            if self.times_out {
                self.timeout = true;
            } else {
                self.rdy = true;
            }
        }
    }

    fn write_cfg(&mut self, v: u32, t: SimTime) {
        let old = self.cfg;
        self.cfg = v & !cali::RDY;
        let rose = |bit: u32| v & bit != 0 && old & bit == 0;
        if rose(cali::START) || rose(cali::START_CYCLING) {
            self.start(t);
        } else if !self.running() {
            self.ends = None;
        }
    }

    fn read_cfg(&self) -> u32 {
        self.cfg | if self.rdy { cali::RDY } else { 0 }
    }

    fn read_cfg1(&self) -> u32 {
        let vld = self.rdy && self.cfg & cali::START_CYCLING != 0;
        (self.value << 7) | u32::from(vld)
    }

    fn read_cfg2(&self) -> u32 {
        (self.cfg2 & !1) | u32::from(self.timeout)
    }
}

/// The general-purpose timer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct GpTimer {
    config: u32,
    value: u64,
    /// Time of the last whole tick counted into `value`.
    anchor: SimTime,
    latched: u64,
    alarm: u64,
    load: u64,
}

impl GpTimer {
    fn new() -> Self {
        Self {
            config: t0::RESET,
            value: 0,
            anchor: SimTime::ZERO,
            latched: 0,
            alarm: 0,
            load: 0,
        }
    }

    fn enabled(&self) -> bool {
        self.config & t0::EN != 0
    }

    fn tick_ps(&self) -> u64 {
        let div = match (self.config >> t0::DIVIDER_SHIFT) & 0xffff {
            0 => 65_536,
            1 => 2,
            d => u64::from(d),
        };
        div * if self.config & t0::USE_XTAL != 0 {
            XTAL_PS
        } else {
            APB_PS
        }
    }

    fn up(&self) -> bool {
        self.config & t0::INCREASE != 0
    }

    /// Count whole ticks up to `t`.
    fn sync_to(&mut self, t: SimTime) {
        if !self.enabled() {
            self.anchor = t;
            return;
        }
        let tick = self.tick_ps();
        let n = t.0.saturating_sub(self.anchor.0) / tick;
        self.value = if self.up() {
            self.value.wrapping_add(n)
        } else {
            self.value.wrapping_sub(n)
        } & MASK54;
        self.anchor = SimTime(self.anchor.0 + n * tick);
    }

    fn next_alarm(&self) -> Option<SimTime> {
        if !self.enabled() || self.config & t0::ALARM_EN == 0 {
            return None;
        }
        let ticks = if self.up() {
            self.alarm.saturating_sub(self.value)
        } else {
            self.value.saturating_sub(self.alarm)
        };
        Some(SimTime(self.anchor.0 + ticks * self.tick_ps()))
    }

    /// Fire the alarm if due by `t`; returns whether it fired.
    fn advance(&mut self, t: SimTime) -> bool {
        let mut fired = false;
        if let Some(at) = self.next_alarm().filter(|&at| at <= t) {
            self.sync_to(at);
            self.config &= !t0::ALARM_EN;
            if self.config & t0::AUTORELOAD != 0 {
                self.value = self.load;
            }
            fired = true;
        }
        self.sync_to(t);
        fired
    }
}

/// One timer group.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timg {
    timer: GpTimer,
    wdt_config: [u32; 6],
    wkey: u32,
    wdt: Stages,
    boot_armed: bool,
    int_ena: u32,
    int_raw: u32,
    regclk: u32,
    cali: Calibration,
}

impl Default for Timg {
    fn default() -> Self {
        Self::new()
    }
}

impl Timg {
    /// Reset state: timer stopped, MWDT off (not armed for flash boot).
    pub fn new() -> Self {
        Self {
            timer: GpTimer::new(),
            wdt_config: [wdt::RESET, 1 << 16, 26_000_000, 0x7ff_ffff, 0xf_ffff, 0xf_ffff],
            wkey: WDT_WKEY,
            wdt: Stages::default(),
            boot_armed: false,
            int_ena: 0,
            int_raw: 0,
            regclk: 1 << 29,
            cali: Calibration::new(),
        }
    }

    /// Arm flash boot protection at `t`, as the ROM does for TIMG0 when it
    /// boots from flash.
    pub fn arm_boot_protection(&mut self, t: SimTime) {
        self.boot_armed = true;
        self.wdt.restart(t);
    }

    /// The MWDT's effective configuration.
    pub fn wdt_timing(&self) -> Timing {
        let c0 = self.wdt_config[0];
        let tick =
            u64::from((self.wdt_config[1] >> 16).max(1)) * if c0 & wdt::USE_XTAL != 0 { XTAL_PS } else { APB_PS };
        let holds_ps = std::array::from_fn(|n| u64::from(self.wdt_config[2 + n]) * tick);
        if c0 & wdt::EN != 0 {
            let actions = std::array::from_fn(|n| match (c0 >> (wdt::STG0_SHIFT - 2 * n as u32)) & 3 {
                0 => Action::Off,
                1 => Action::Interrupt,
                2 => Action::Reset(ResetKind::Cpu),
                _ => Action::Reset(ResetKind::System),
            });
            Timing {
                enabled: true,
                actions,
                holds_ps,
            }
        } else {
            Timing {
                enabled: self.boot_armed && c0 & wdt::FLASHBOOT_MOD_EN != 0,
                actions: [Action::Reset(ResetKind::System), Action::Off, Action::Off, Action::Off],
                holds_ps,
            }
        }
    }

    /// Time of the next alarm or watchdog stage expiry.
    pub fn next_event(&self) -> Option<SimTime> {
        [
            self.timer.next_alarm(),
            self.wdt.next(&self.wdt_timing()),
            self.cali.next_event(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    /// Process everything due at or before `t`. Returns a watchdog reset,
    /// if one happened (state after it is moot: the chip resets).
    pub fn advance(&mut self, t: SimTime) -> Option<(SimTime, ResetKind)> {
        self.cali.advance(t);
        if self.timer.advance(t) {
            self.int_raw |= 1;
        }
        let out = self.wdt.advance(t, &self.wdt_timing());
        if out.interrupt {
            self.int_raw |= 2;
        }
        out.reset
    }

    /// `INT_ST`: bit 0 timer, bit 1 watchdog.
    pub fn int_status(&self) -> u32 {
        self.int_raw & self.int_ena
    }

    /// Whether `off` is a modelled register.
    pub fn is_reg(off: u32) -> bool {
        let wdt_cali_int = (reg::WDTCONFIG0..=reg::RTCCALICFG2).contains(&off);
        off.is_multiple_of(4) && (off <= reg::T0LOAD || wdt_cali_int || off >= reg::NTIMERS_DATE)
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        let t = &self.timer;
        Some(match off {
            reg::T0CONFIG => t.config,
            reg::T0LO => t.latched as u32,
            reg::T0HI => (t.latched >> 32) as u32,
            reg::T0UPDATE | reg::T0LOAD | reg::WDTFEED | reg::INT_CLR => 0,
            reg::T0ALARMLO => t.alarm as u32,
            reg::T0ALARMHI => (t.alarm >> 32) as u32,
            reg::T0LOADLO => t.load as u32,
            reg::T0LOADHI => (t.load >> 32) as u32,
            reg::WDTCONFIG0..=reg::WDTCONFIG5 => self.wdt_config[((off - reg::WDTCONFIG0) / 4) as usize],
            reg::WDTWPROTECT => self.wkey,
            reg::INT_ENA => self.int_ena,
            reg::INT_RAW => self.int_raw,
            reg::INT_ST => self.int_status(),
            reg::RTCCALICFG => self.cali.read_cfg(),
            reg::RTCCALICFG1 => self.cali.read_cfg1(),
            reg::RTCCALICFG2 => self.cali.read_cfg2(),
            reg::NTIMERS_DATE => 0x200_6191,
            reg::REGCLK => self.regclk,
            _ => return None,
        })
    }

    /// Write a register at `t` (after [`Self::advance`] to `t`).
    pub fn write(&mut self, off: u32, v: u32, t: SimTime) -> Option<()> {
        let unlocked = self.wkey == WDT_WKEY;
        let tm = &mut self.timer;
        match off {
            reg::T0CONFIG => {
                tm.sync_to(t);
                let old = tm.config;
                let new = v & !t0::DIVCNT_RST;
                let clock_bits = t0::USE_XTAL | (0xffff << t0::DIVIDER_SHIFT);
                let started = new & t0::EN != 0 && old & t0::EN == 0;
                if started || (old ^ new) & clock_bits != 0 || v & t0::DIVCNT_RST != 0 {
                    tm.anchor = t;
                }
                tm.config = new;
            }
            reg::T0LO | reg::T0HI => {}
            reg::T0UPDATE => {
                tm.sync_to(t);
                tm.latched = tm.value;
            }
            reg::T0ALARMLO => tm.alarm = (tm.alarm & !0xffff_ffff) | u64::from(v),
            reg::T0ALARMHI => tm.alarm = (tm.alarm & 0xffff_ffff) | (u64::from(v & 0x3f_ffff) << 32),
            reg::T0LOADLO => tm.load = (tm.load & !0xffff_ffff) | u64::from(v),
            reg::T0LOADHI => tm.load = (tm.load & 0xffff_ffff) | (u64::from(v & 0x3f_ffff) << 32),
            reg::T0LOAD => {
                tm.sync_to(t);
                tm.value = tm.load;
                tm.anchor = t;
            }
            reg::WDTCONFIG0..=reg::WDTCONFIG5 if unlocked => {
                let i = ((off - reg::WDTCONFIG0) / 4) as usize;
                let was = self.wdt_timing().enabled;
                self.wdt_config[i] = if i == 0 { v & !wdt::CONF_UPDATE_EN } else { v };
                if self.wdt_timing().enabled && !was {
                    self.wdt.restart(t);
                }
            }
            reg::WDTFEED if unlocked => self.wdt.restart(t),
            reg::WDTCONFIG0..=reg::WDTFEED => {} // write-protected
            reg::WDTWPROTECT => self.wkey = v,
            reg::INT_ENA => self.int_ena = v & 3,
            reg::INT_RAW | reg::INT_CLR => self.int_raw &= !v,
            reg::RTCCALICFG => self.cali.write_cfg(v, t),
            reg::RTCCALICFG2 => self.cali.cfg2 = v & !0b111,
            reg::INT_ST | reg::NTIMERS_DATE | reg::RTCCALICFG1 => {}
            reg::REGCLK => self.regclk = v,
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(g: &mut Timg, off: u32, v: u32, t: SimTime) -> Option<(SimTime, ResetKind)> {
        let r = g.advance(t);
        g.write(off, v, t).unwrap();
        r
    }

    fn read_count(g: &mut Timg, t: SimTime) -> u64 {
        w(g, reg::T0UPDATE, 0, t);
        u64::from(g.read(reg::T0LO).unwrap()) | (u64::from(g.read(reg::T0HI).unwrap()) << 32)
    }

    const fn us(n: u64) -> SimTime {
        SimTime(n * 1_000_000)
    }

    #[test]
    fn reset_values_match_the_trm() {
        let g = Timg::new();
        assert_eq!(g.read(reg::T0CONFIG), Some(0x6000_2000));
        assert_eq!(g.read(reg::WDTCONFIG0), Some(0x0004_c000));
        assert_eq!(g.read(reg::WDTCONFIG1), Some(0x0001_0000));
        assert_eq!(g.read(reg::WDTCONFIG2), Some(26_000_000));
        assert_eq!(g.read(reg::WDTWPROTECT), Some(0x50d8_3aa1));
        assert_eq!(g.next_event(), None, "nothing runs at reset");
        assert!(Timg::is_reg(reg::T0LOAD) && Timg::is_reg(reg::INT_CLR) && Timg::is_reg(reg::RTCCALICFG));
    }

    #[test]
    fn counter_runs_at_apb_over_the_divider() {
        let mut g = Timg::new();
        // Divider 80: 1 MHz.
        w(
            &mut g,
            reg::T0CONFIG,
            t0::EN | t0::INCREASE | (80 << t0::DIVIDER_SHIFT),
            us(5),
        );
        assert_eq!(read_count(&mut g, us(1005)), 1000);
        assert_eq!(g.read(reg::T0UPDATE), Some(0), "update self-clears");
        // Divider 0 = 65536, XTAL: one tick per 1.6384 ms.
        w(&mut g, reg::T0CONFIG, t0::EN | t0::INCREASE | t0::USE_XTAL, us(1005));
        assert_eq!(read_count(&mut g, SimTime(us(1005).0 + 3 * 1_638_400_000)), 1003);
        // Divider 1 counts as 2: 40 MHz from APB.
        w(
            &mut g,
            reg::T0CONFIG,
            t0::EN | t0::INCREASE | (1 << t0::DIVIDER_SHIFT),
            us(10_000),
        );
        let base = read_count(&mut g, us(10_000));
        assert_eq!(read_count(&mut g, us(10_001)), base + 40);
        // Counting down, stopping.
        w(&mut g, reg::T0CONFIG, t0::EN | (1 << t0::DIVIDER_SHIFT), us(10_001));
        assert_eq!(read_count(&mut g, us(10_002)), base);
        w(&mut g, reg::T0CONFIG, 1 << t0::DIVIDER_SHIFT, us(10_002));
        assert_eq!(read_count(&mut g, us(20_000)), base);
    }

    #[test]
    fn load_sets_the_counter() {
        let mut g = Timg::new();
        w(&mut g, reg::T0LOADLO, 7, us(0));
        w(&mut g, reg::T0LOADHI, 0xffff_ffff, us(0));
        w(&mut g, reg::T0LOAD, 1, us(0));
        assert_eq!(read_count(&mut g, us(1)), (0x3f_ffff << 32) | 7, "54-bit");
    }

    #[test]
    fn alarm_with_autoreload_repeats_when_rearmed() {
        let mut g = Timg::new();
        let div80 = t0::EN | t0::INCREASE | t0::AUTORELOAD | (80 << t0::DIVIDER_SHIFT);
        w(&mut g, reg::T0ALARMLO, 1000, us(0));
        w(&mut g, reg::INT_ENA, 1, us(0));
        w(&mut g, reg::T0CONFIG, div80 | t0::ALARM_EN, us(0));
        assert_eq!(g.next_event(), Some(us(1000)));
        g.advance(us(1000));
        assert_eq!(g.int_status(), 1);
        assert_eq!(g.read(reg::T0CONFIG).unwrap() & t0::ALARM_EN, 0, "ALARM_EN self-clears");
        assert_eq!(g.next_event(), None);
        assert_eq!(read_count(&mut g, us(1000)), 0, "reloaded from T0LOAD (0)");
        // ISR: clear, re-arm at 1003 µs; next alarm 1000 ticks after the reload.
        w(&mut g, reg::INT_CLR, 1, us(1003));
        assert_eq!(g.int_status(), 0);
        w(&mut g, reg::T0CONFIG, div80 | t0::ALARM_EN, us(1003));
        assert_eq!(g.next_event(), Some(us(2000)), "no drift from ISR latency");
    }

    #[test]
    fn alarm_in_the_past_fires_at_once() {
        let mut g = Timg::new();
        w(
            &mut g,
            reg::T0CONFIG,
            t0::EN | t0::INCREASE | (80 << t0::DIVIDER_SHIFT),
            us(0),
        );
        w(&mut g, reg::T0ALARMLO, 10, us(50));
        w(
            &mut g,
            reg::T0CONFIG,
            t0::EN | t0::INCREASE | (80 << t0::DIVIDER_SHIFT) | t0::ALARM_EN,
            us(50),
        );
        assert_eq!(g.next_event(), Some(us(50)));
        g.advance(us(50));
        assert_eq!(g.read(reg::INT_RAW), Some(1));
        assert_eq!(g.int_status(), 0, "not enabled");
        // Counting down to the alarm.
        w(&mut g, reg::INT_RAW, 1, us(50));
        w(&mut g, reg::T0ALARMLO, 0, us(50));
        w(
            &mut g,
            reg::T0CONFIG,
            t0::EN | (80 << t0::DIVIDER_SHIFT) | t0::ALARM_EN,
            us(50),
        );
        assert_eq!(g.next_event(), Some(us(100)), "count was 50 (no reload)");
    }

    #[test]
    fn mwdt_stages_interrupt_then_reset() {
        let mut g = Timg::new();
        // Prescale 80 (1 µs ticks), stage 0 interrupt after 100, stage 1 CPU reset after 50.
        w(&mut g, reg::WDTCONFIG1, 80 << 16, us(0));
        w(&mut g, reg::WDTCONFIG2, 100, us(0));
        w(&mut g, reg::WDTCONFIG2 + 4, 50, us(0));
        w(&mut g, reg::INT_ENA, 2, us(0));
        let cfg = wdt::EN | (1 << wdt::STG0_SHIFT) | (2 << (wdt::STG0_SHIFT - 2)) | wdt::CONF_UPDATE_EN;
        w(&mut g, reg::WDTCONFIG0, cfg, us(10));
        assert_eq!(g.read(reg::WDTCONFIG0).unwrap() & wdt::CONF_UPDATE_EN, 0);
        assert_eq!(g.next_event(), Some(us(110)));
        // Feeding restarts stage 0.
        w(&mut g, reg::WDTFEED, 1, us(100));
        assert_eq!(g.next_event(), Some(us(200)));
        assert_eq!(g.advance(us(200)), None);
        assert_eq!(g.int_status(), 2, "stage 0: interrupt");
        assert_eq!(g.advance(us(300)), Some((us(250), ResetKind::Cpu)));
    }

    #[test]
    fn write_protection_and_flash_boot_disable() {
        let mut g = Timg::new();
        g.arm_boot_protection(us(0));
        // Flash boot protection: system reset after 26e6 x 12.5 ns = 325 ms.
        assert_eq!(g.next_event(), Some(SimTime(325_000_000_000)));
        // Lock, then try to disable: ignored.
        w(&mut g, reg::WDTWPROTECT, 0, us(1));
        w(&mut g, reg::WDTCONFIG0, 0, us(2));
        w(&mut g, reg::WDTFEED, 1, us(2));
        assert_eq!(g.read(reg::WDTCONFIG0), Some(wdt::RESET));
        assert_eq!(g.next_event(), Some(SimTime(325_000_000_000)));
        // Unlock with the key, clear FLASHBOOT_MOD_EN, re-lock: disabled.
        w(&mut g, reg::WDTWPROTECT, WDT_WKEY, us(3));
        w(&mut g, reg::WDTCONFIG0, wdt::CONF_UPDATE_EN, us(3));
        w(&mut g, reg::WDTWPROTECT, 0, us(3));
        assert_eq!(g.next_event(), None);
        assert_eq!(g.advance(SimTime(1_000_000_000_000)), None);
    }

    #[test]
    fn rtc_calibration_counts_xtal_cycles_over_slow_cycles() {
        let mut g = Timg::new();
        assert_eq!(g.read(reg::RTCCALICFG), Some(cali::RESET), "cycling, not ready yet");
        assert_eq!(g.read(reg::RTCCALICFG2), Some(cali::CFG2_RESET));
        // Cycling from reset: one RC_FAST/256 cycle, 585 XTAL cycles.
        g.advance(us(20));
        assert_eq!(g.read(reg::RTCCALICFG), Some(cali::RESET | cali::RDY));
        assert_eq!(
            g.read(reg::RTCCALICFG1),
            Some((40_000_000 / RC_FAST_DIV_HZ as u32) << 7 | 1)
        );
        // One-shot: 1024 RC_SLOW cycles (7.53 ms) as esp-hal does it.
        w(&mut g, reg::RTCCALICFG, cali::RESET & !cali::START_CYCLING, us(100));
        w(&mut g, reg::RTCCALICFG, 1024 << cali::MAX_SHIFT, us(100));
        w(&mut g, reg::RTCCALICFG, 1024 << cali::MAX_SHIFT | cali::START, us(100));
        let n = 1024 * 40_000_000 / RC_SLOW_HZ;
        let end = SimTime(us(100).0 + n * XTAL_PS);
        assert_eq!(g.next_event(), Some(end));
        g.advance(SimTime(end.0 - 1));
        assert_eq!(g.read(reg::RTCCALICFG).unwrap() & cali::RDY, 0);
        g.advance(end);
        assert_ne!(g.read(reg::RTCCALICFG).unwrap() & cali::RDY, 0);
        assert_eq!(g.read(reg::RTCCALICFG1), Some((n as u32) << 7), "no cycling data");
        assert_eq!(g.next_event(), None);
    }

    #[test]
    fn rtc_calibration_times_out_without_a_32k_crystal() {
        let mut g = Timg::new();
        w(&mut g, reg::RTCCALICFG, 0, us(0));
        w(&mut g, reg::RTCCALICFG2, 1000 << 7, us(0));
        let cfg = 2 << cali::CLK_SEL_SHIFT | 100 << cali::MAX_SHIFT;
        w(&mut g, reg::RTCCALICFG, cfg | cali::START, us(1));
        g.advance(SimTime(us(1).0 + 1000 * XTAL_PS));
        assert_eq!(g.read(reg::RTCCALICFG2), Some(1000 << 7 | 1));
        assert_eq!(g.read(reg::RTCCALICFG).unwrap() & cali::RDY, 0);
    }

    #[test]
    fn armed_flash_boot_resets_the_system() {
        let mut g = Timg::new();
        g.arm_boot_protection(us(0));
        assert_eq!(
            g.advance(SimTime(400_000_000_000)),
            Some((SimTime(325_000_000_000), ResetKind::System))
        );
    }
}
