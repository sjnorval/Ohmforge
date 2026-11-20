//! The watchdogs in RTC_CNTL (TRM chapters 9 and 12): the RTC watchdog
//! (RWDT) and the super watchdog (SWD), plus the RTC interrupt registers
//! they use. Other RTC_CNTL registers are not modelled (they read back).
//!
//! **RWDT.** Like the MWDT (see [`crate::watchdog`]) but timed by the RTC
//! slow clock, here the internal 136 kHz RC oscillator (nominal; the real
//! one drifts and is calibrated by firmware). Stage actions: 0 off,
//! 1 interrupt (`INT_RAW.WDT`, bit 3, routed to the `RTC_CORE` source),
//! 2 CPU reset, 3 core reset, 4 system reset. Configuration and feed writes
//! need the key `0x50D8_3AA1` in `WDTWPROTECT` (its reset value). Feed with
//! `WDTFEED.WDT_FEED` (bit 31). In flash boot the ROM leaves it armed
//! through `WDT_FLASHBOOT_MOD_EN`: when armed ([`RtcCntl::arm_boot_watchdogs`])
//! and that bit is set, stage 0 resets the system after `WDT_STG0_HOLD`
//! slow-clock ticks (200000, about 1.5 s) even with `WDT_EN` clear.
//!
//! **SWD.** Resets the system unless fed (`SWD_FEED`, bit 29) at least
//! every [`SWD_TIMEOUT`] (about one second in the TRM; modelled as exactly
//! 1 s), disabled (`SWD_DISABLE`, bit 30) or set to feed itself
//! (`SWD_AUTO_FEED_EN`, bit 31). `SWD_CONF` writes need the key
//! `0x8F1D_312A` in `SWD_WPROTECT`. It runs only when armed by
//! [`RtcCntl::arm_boot_watchdogs`]: with direct boot the bootloader's own
//! handling of it (auto-feed) is assumed. Its reset flag is not modelled.

use of_core::SimTime;

use crate::watchdog::{Action, ResetKind, Stages, Timing};

/// RTC slow clock (internal RC oscillator, nominal).
pub const SLOW_CLK_HZ: u64 = 136_000;
/// One slow-clock tick in picoseconds (rounded down).
pub const SLOW_TICK_PS: u64 = 1_000_000_000_000 / SLOW_CLK_HZ;
/// The RWDT write-protection key.
pub const WDT_WKEY: u32 = 0x50d8_3aa1;
/// The SWD write-protection key.
pub const SWD_WKEY: u32 = 0x8f1d_312a;
/// Super watchdog timeout.
pub const SWD_TIMEOUT: SimTime = SimTime(1_000_000_000_000);

/// `RTC_CNTL_RESET_STATE_REG` fields.
pub mod reset_state {
    /// `STAT_VECTOR_SEL_PROCPU` and `_APPCPU` (reset value 1).
    pub const STAT_VECTOR_SEL: u32 = 0b11 << 12;
    /// Read/write bits: reset masks, OCD halt, vector select.
    pub const RW: u32 = (0b11 << 24) | (0b11 << 18) | STAT_VECTOR_SEL;
}

/// Reset cause codes (ESP-IDF `soc/reset_reasons.h`).
pub mod reset_cause {
    /// Chip power-on (also brown-out and super watchdog chip resets).
    pub const POWER_ON: u32 = 0x01;
    /// MWDT0 reset the digital core.
    pub const CORE_MWDT0: u32 = 0x07;
    /// MWDT1 reset the digital core.
    pub const CORE_MWDT1: u32 = 0x08;
    /// RTC watchdog reset the digital core.
    pub const CORE_RTC_WDT: u32 = 0x09;
    /// MWDT0 reset the CPU.
    pub const CPU0_MWDT0: u32 = 0x0b;
    /// RTC watchdog reset the CPU.
    pub const CPU0_RTC_WDT: u32 = 0x0d;
    /// RTC watchdog reset the digital core and RTC.
    pub const SYS_RTC_WDT: u32 = 0x10;
    /// MWDT1 reset the CPU.
    pub const CPU0_MWDT1: u32 = 0x11;
    /// Super watchdog reset the digital core and RTC.
    pub const SYS_SUPER_WDT: u32 = 0x12;
}

/// Register offsets from [`crate::map::RTC_CNTL_BASE`].
pub mod reg {
    /// Reset state: reset cause (bits 5:0) and flags.
    pub const RESET_STATE: u32 = 0x38;
    /// RTC interrupt enable (bit 3: RWDT).
    pub const INT_ENA: u32 = 0x40;
    /// RTC raw interrupt status.
    pub const INT_RAW: u32 = 0x44;
    /// RTC masked interrupt status.
    pub const INT_ST: u32 = 0x48;
    /// RTC interrupt clear.
    pub const INT_CLR: u32 = 0x4c;
    /// RWDT configuration (see [`super::wdt`]).
    pub const WDTCONFIG0: u32 = 0x90;
    /// Stage 0 timeout in slow-clock ticks (stages 1 to 3 at +4).
    pub const WDTCONFIG1: u32 = 0x94;
    /// Stage 3 timeout.
    pub const WDTCONFIG4: u32 = 0xa0;
    /// Feed (bit 31).
    pub const WDTFEED: u32 = 0xa4;
    /// RWDT write-protection key.
    pub const WDTWPROTECT: u32 = 0xa8;
    /// Super watchdog configuration (see [`super::swd`]).
    pub const SWD_CONF: u32 = 0xac;
    /// SWD write-protection key.
    pub const SWD_WPROTECT: u32 = 0xb0;
}

/// `WDTCONFIG0` bits.
pub mod wdt {
    /// RWDT enable.
    pub const EN: u32 = 1 << 31;
    /// Stage 0 action shift (3 bits); stage n is at `STG0_SHIFT - 3n`.
    pub const STG0_SHIFT: u32 = 28;
    /// Flash boot protection.
    pub const FLASHBOOT_MOD_EN: u32 = 1 << 12;
    /// Reset value.
    pub const RESET: u32 = (1 << 16) | (1 << 13) | FLASHBOOT_MOD_EN | (1 << 9) | 20;
    /// `WDTFEED` feed bit.
    pub const FEED: u32 = 1 << 31;
}

/// `SWD_CONF` bits.
pub mod swd {
    /// Feed automatically (the SWD never fires).
    pub const AUTO_FEED_EN: u32 = 1 << 31;
    /// Disable.
    pub const DISABLE: u32 = 1 << 30;
    /// Feed (write-only).
    pub const FEED: u32 = 1 << 29;
    /// Clear the reset flag (write-only).
    pub const RST_FLAG_CLR: u32 = 1 << 28;
    /// Reset value (signal width 300).
    pub const RESET: u32 = 300 << 18;
}

const INT_WDT: u32 = 1 << 3;

/// Which RTC_CNTL watchdog reset the chip.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RtcWatchdog {
    /// The RTC watchdog.
    Rwdt,
    /// The super watchdog.
    Swd,
}

/// RTC_CNTL watchdog state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RtcCntl {
    int_ena: u32,
    int_raw: u32,
    wdt_config: [u32; 5],
    wkey: u32,
    wdt: Stages,
    swd_conf: u32,
    swd_wkey: u32,
    swd_fed: SimTime,
    boot_armed: bool,
    reset_state: u32,
}

impl Default for RtcCntl {
    fn default() -> Self {
        Self::new()
    }
}

impl RtcCntl {
    /// Reset state, watchdogs not armed.
    pub fn new() -> Self {
        Self {
            int_ena: 0,
            int_raw: 0,
            wdt_config: [wdt::RESET, 200_000, 80_000, 0xfff, 0xfff],
            wkey: WDT_WKEY,
            wdt: Stages::default(),
            swd_conf: swd::RESET,
            swd_wkey: SWD_WKEY,
            swd_fed: SimTime::ZERO,
            boot_armed: false,
            // Reset cause 1 (power-on), both STAT_VECTOR_SEL bits set.
            reset_state: reset_state::STAT_VECTOR_SEL | reset_cause::POWER_ON,
        }
    }

    /// Record why the chip last reset (`RTC_CNTL_RESET_CAUSE_PROCPU`, one
    /// of the [`reset_cause`] codes); the ROM prints it as `rst:0x..`.
    pub fn set_reset_cause(&mut self, cause: u32) {
        self.reset_state = (self.reset_state & !0x3f) | (cause & 0x3f);
    }

    /// The reset cause code.
    pub fn reset_cause(&self) -> u32 {
        self.reset_state & 0x3f
    }

    /// Read a register that does not depend on time (reset state).
    pub fn read_static(&self, off: u32) -> Option<u32> {
        (off == reg::RESET_STATE).then_some(self.reset_state)
    }

    /// Write a register that does not depend on time (reset state: only
    /// the read/write bits change; the flag-clear bits are write-only).
    pub fn write_static(&mut self, off: u32, v: u32) -> Option<()> {
        (off == reg::RESET_STATE).then(|| {
            self.reset_state = (self.reset_state & !reset_state::RW) | (v & reset_state::RW);
        })
    }

    /// Arm the RWDT flash boot protection and start the SWD at `t`, as
    /// after a power-on reset into flash boot.
    pub fn arm_boot_watchdogs(&mut self, t: SimTime) {
        self.boot_armed = true;
        self.wdt.restart(t);
        self.swd_fed = t;
    }

    /// The RWDT's effective configuration.
    pub fn wdt_timing(&self) -> Timing {
        let c0 = self.wdt_config[0];
        let holds_ps = std::array::from_fn(|n| u64::from(self.wdt_config[1 + n]) * SLOW_TICK_PS);
        if c0 & wdt::EN != 0 {
            let actions = std::array::from_fn(|n| match (c0 >> (wdt::STG0_SHIFT - 3 * n as u32)) & 7 {
                1 => Action::Interrupt,
                2 => Action::Reset(ResetKind::Cpu),
                3 => Action::Reset(ResetKind::Core),
                4 => Action::Reset(ResetKind::System),
                _ => Action::Off,
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

    fn swd_deadline(&self) -> Option<SimTime> {
        let running = self.boot_armed && self.swd_conf & (swd::DISABLE | swd::AUTO_FEED_EN) == 0;
        running.then(|| SimTime(self.swd_fed.0 + SWD_TIMEOUT.0))
    }

    /// Time of the next watchdog stage expiry.
    pub fn next_event(&self) -> Option<SimTime> {
        [self.wdt.next(&self.wdt_timing()), self.swd_deadline()]
            .into_iter()
            .flatten()
            .min()
    }

    /// Process everything due at or before `t`; returns the first reset.
    pub fn advance(&mut self, t: SimTime) -> Option<(SimTime, ResetKind, RtcWatchdog)> {
        let out = self.wdt.advance(t, &self.wdt_timing());
        if out.interrupt {
            self.int_raw |= INT_WDT;
        }
        let rwdt = out.reset.map(|(at, kind)| (at, kind, RtcWatchdog::Rwdt));
        let swd = self
            .swd_deadline()
            .filter(|&at| at <= t)
            .map(|at| (at, ResetKind::System, RtcWatchdog::Swd));
        [rwdt, swd].into_iter().flatten().min_by_key(|r| r.0)
    }

    /// Whether the RTC_CORE interrupt source is asserted.
    pub fn irq(&self) -> bool {
        self.int_raw & self.int_ena != 0
    }

    /// Whether `off` is a modelled register.
    pub fn is_reg(off: u32) -> bool {
        off.is_multiple_of(4)
            && ((reg::INT_ENA..=reg::INT_CLR).contains(&off) || (reg::WDTCONFIG0..=reg::SWD_WPROTECT).contains(&off))
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        Some(match off {
            reg::INT_ENA => self.int_ena,
            reg::INT_RAW => self.int_raw,
            reg::INT_ST => self.int_raw & self.int_ena,
            reg::INT_CLR | reg::WDTFEED => 0,
            reg::WDTCONFIG0..=reg::WDTCONFIG4 => self.wdt_config[((off - reg::WDTCONFIG0) / 4) as usize],
            reg::WDTWPROTECT => self.wkey,
            reg::SWD_CONF => self.swd_conf,
            reg::SWD_WPROTECT => self.swd_wkey,
            _ => return None,
        })
    }

    /// Write a register at `t` (after [`Self::advance`] to `t`).
    pub fn write(&mut self, off: u32, v: u32, t: SimTime) -> Option<()> {
        let unlocked = self.wkey == WDT_WKEY;
        match off {
            reg::INT_ENA => self.int_ena = v,
            reg::INT_RAW | reg::INT_ST => {}
            reg::INT_CLR => self.int_raw &= !v,
            reg::WDTCONFIG0..=reg::WDTCONFIG4 if unlocked => {
                let was = self.wdt_timing().enabled;
                self.wdt_config[((off - reg::WDTCONFIG0) / 4) as usize] = v;
                if self.wdt_timing().enabled && !was {
                    self.wdt.restart(t);
                }
            }
            reg::WDTFEED if unlocked && v & wdt::FEED != 0 => self.wdt.restart(t),
            reg::WDTCONFIG0..=reg::WDTFEED => {} // write-protected or no feed bit
            reg::WDTWPROTECT => self.wkey = v,
            reg::SWD_CONF if self.swd_wkey == SWD_WKEY => {
                if v & swd::FEED != 0 {
                    self.swd_fed = t;
                }
                let was_running = self.swd_deadline().is_some();
                self.swd_conf = v & !(swd::FEED | swd::RST_FLAG_CLR | 3);
                if !was_running {
                    self.swd_fed = t;
                }
            }
            reg::SWD_CONF => {}
            reg::SWD_WPROTECT => self.swd_wkey = v,
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(r: &mut RtcCntl, off: u32, v: u32, t: SimTime) {
        assert_eq!(r.advance(t), None);
        r.write(off, v, t).unwrap();
    }

    const fn ms(n: u64) -> SimTime {
        SimTime(n * 1_000_000_000)
    }

    #[test]
    fn reset_values_and_nothing_runs_unarmed() {
        let r = RtcCntl::new();
        assert_eq!(r.read(reg::WDTCONFIG0), Some(0x0001_3214));
        assert_eq!(r.read(reg::WDTCONFIG1), Some(200_000));
        assert_eq!(r.read(reg::WDTWPROTECT), Some(WDT_WKEY));
        assert_eq!(r.read(reg::SWD_CONF), Some(0x04b0_0000));
        assert_eq!(r.next_event(), None);
        assert!(RtcCntl::is_reg(0xb0) && !RtcCntl::is_reg(0x38) && !RtcCntl::is_reg(0xb4));
    }

    #[test]
    fn armed_boot_watchdogs_reset_unless_disabled() {
        let mut r = RtcCntl::new();
        r.arm_boot_watchdogs(SimTime::ZERO);
        // The SWD (1 s) comes before the RWDT (200000 / 136 kHz = 1.47 s).
        assert_eq!(r.next_event(), Some(ms(1000)));
        assert_eq!(
            r.advance(ms(2000)),
            Some((ms(1000), ResetKind::System, RtcWatchdog::Swd))
        );

        let mut r = RtcCntl::new();
        r.arm_boot_watchdogs(SimTime::ZERO);
        // Firmware feeds the SWD every 500 ms: the RWDT fires instead.
        for n in 1..=2 {
            w(&mut r, reg::SWD_CONF, swd::RESET | swd::FEED, ms(500 * n));
        }
        let rwdt = SimTime(200_000 * SLOW_TICK_PS);
        assert_eq!(r.advance(ms(1600)), Some((rwdt, ResetKind::System, RtcWatchdog::Rwdt)));

        // The usual start-up sequence: unlock, disable, lock.
        let mut r = RtcCntl::new();
        r.arm_boot_watchdogs(SimTime::ZERO);
        w(&mut r, reg::WDTWPROTECT, WDT_WKEY, ms(1));
        w(&mut r, reg::WDTCONFIG0, 0, ms(1));
        w(&mut r, reg::WDTWPROTECT, 0, ms(1));
        w(&mut r, reg::SWD_WPROTECT, SWD_WKEY, ms(1));
        w(&mut r, reg::SWD_CONF, swd::RESET | swd::AUTO_FEED_EN, ms(1));
        w(&mut r, reg::SWD_WPROTECT, 0, ms(1));
        assert_eq!(r.next_event(), None);
        assert_eq!(r.advance(ms(10_000)), None);
    }

    #[test]
    fn locked_registers_ignore_writes() {
        let mut r = RtcCntl::new();
        r.arm_boot_watchdogs(SimTime::ZERO);
        w(&mut r, reg::SWD_WPROTECT, 0, ms(1));
        w(&mut r, reg::SWD_CONF, swd::DISABLE, ms(1));
        assert_eq!(r.read(reg::SWD_CONF), Some(swd::RESET));
        w(&mut r, reg::WDTWPROTECT, 0, ms(1));
        w(&mut r, reg::WDTCONFIG0, 0, ms(1));
        assert_eq!(r.read(reg::WDTCONFIG0), Some(wdt::RESET));
        assert_eq!(r.next_event(), Some(ms(1000)));
    }

    #[test]
    fn rwdt_interrupt_stage_and_feed() {
        let mut r = RtcCntl::new();
        w(&mut r, reg::WDTCONFIG1, 136, ms(0)); // 1 ms
        w(&mut r, reg::WDTCONFIG1 + 4, 136 * 5, ms(0));
        let cfg = wdt::EN | (1 << wdt::STG0_SHIFT) | (3 << (wdt::STG0_SHIFT - 3));
        w(&mut r, reg::WDTCONFIG0, cfg, ms(0));
        w(&mut r, reg::INT_ENA, INT_WDT, ms(0));
        w(&mut r, reg::WDTFEED, 0, ms(0)); // no feed bit: ignored
        assert_eq!(r.next_event(), Some(SimTime(136 * SLOW_TICK_PS)));
        w(&mut r, reg::WDTFEED, wdt::FEED, SimTime(500_000_000));
        assert_eq!(r.next_event(), Some(SimTime(500_000_000 + 136 * SLOW_TICK_PS)));
        assert_eq!(r.advance(ms(2)), None);
        assert!(r.irq());
        assert_eq!(r.read(reg::INT_ST), Some(INT_WDT));
        r.write(reg::INT_CLR, INT_WDT, ms(2)).unwrap();
        assert!(!r.irq());
        let core = r.advance(ms(10)).unwrap();
        assert_eq!(core.1, ResetKind::Core);
    }
}
