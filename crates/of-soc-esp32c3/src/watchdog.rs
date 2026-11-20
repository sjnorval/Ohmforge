//! The stage machine shared by the ESP32-C3 watchdogs (the MWDT in each
//! timer group and the RTC watchdog; TRM chapter 12, "Watchdog Timers").
//!
//! A running watchdog counts through stages 0 to 3. When the count reaches
//! the current stage's hold time, that stage's action is taken and the next
//! stage starts (stage 3 wraps to 0). Feeding restarts stage 0.

use of_core::SimTime;

/// What a watchdog (or other reset source) resets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ResetKind {
    /// The CPU core only; peripherals keep their state.
    Cpu,
    /// The digital core: CPU and digital peripherals, not the RTC domain.
    Core,
    /// The whole chip, RTC domain included.
    System,
}

/// Which watchdog caused a reset.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResetSource {
    /// The MWDT of timer group 0 or 1.
    Mwdt(u8),
    /// The RTC watchdog.
    Rwdt,
    /// The super watchdog.
    Swd,
}

/// A watchdog reset: when, what and who.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WatchdogReset {
    /// When the stage expired.
    pub at: SimTime,
    /// What is reset.
    pub kind: ResetKind,
    /// Which watchdog.
    pub source: ResetSource,
}

impl WatchdogReset {
    /// The reset cause code the chip reports afterwards in
    /// `RTC_CNTL_RESET_STATE_REG` (and the ROM prints as `rst:0x..`).
    pub fn cause_code(&self) -> u32 {
        use crate::rtc_cntl::reset_cause as c;
        match (self.source, self.kind) {
            (ResetSource::Mwdt(0), ResetKind::Cpu) => c::CPU0_MWDT0,
            (ResetSource::Mwdt(_), ResetKind::Cpu) => c::CPU0_MWDT1,
            (ResetSource::Mwdt(0), _) => c::CORE_MWDT0,
            (ResetSource::Mwdt(_), _) => c::CORE_MWDT1,
            (ResetSource::Rwdt, ResetKind::Cpu) => c::CPU0_RTC_WDT,
            (ResetSource::Rwdt, ResetKind::Core) => c::CORE_RTC_WDT,
            (ResetSource::Rwdt, ResetKind::System) => c::SYS_RTC_WDT,
            (ResetSource::Swd, _) => c::SYS_SUPER_WDT,
        }
    }
}

/// What a stage does when it expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    /// Nothing; go on to the next stage.
    Off,
    /// Raise the watchdog interrupt.
    Interrupt,
    /// Reset.
    Reset(ResetKind),
}

/// A watchdog's effective configuration, decoded from its registers.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// Whether the watchdog runs.
    pub enabled: bool,
    /// Per-stage action.
    pub actions: [Action; 4],
    /// Per-stage timeout in picoseconds (at least 1).
    pub holds_ps: [u64; 4],
}

/// Stage-machine state.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Stages {
    stage: u8,
    start: SimTime,
}

/// What [`Stages::advance`] saw.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Outcome {
    /// An interrupt stage expired.
    pub interrupt: bool,
    /// A reset stage expired: when and what. Processing stops there.
    pub reset: Option<(SimTime, ResetKind)>,
}

impl Stages {
    /// Current stage (0 to 3).
    pub fn stage(&self) -> u8 {
        self.stage
    }

    /// Restart stage 0 at `t` (feed, enable).
    pub fn restart(&mut self, t: SimTime) {
        self.stage = 0;
        self.start = t;
    }

    /// When the current stage expires, if the watchdog can do anything.
    pub fn next(&self, cfg: &Timing) -> Option<SimTime> {
        if !cfg.enabled || cfg.actions.iter().all(|a| *a == Action::Off) {
            return None;
        }
        Some(SimTime(self.start.0 + cfg.holds_ps[usize::from(self.stage)].max(1)))
    }

    /// Take every stage expiry at or before `t`.
    pub fn advance(&mut self, t: SimTime, cfg: &Timing) -> Outcome {
        let mut out = Outcome::default();
        while let Some(at) = self.next(cfg).filter(|&at| at <= t) {
            let action = cfg.actions[usize::from(self.stage)];
            self.stage = (self.stage + 1) % 4;
            self.start = at;
            match action {
                Action::Off => {}
                Action::Interrupt => out.interrupt = true,
                Action::Reset(kind) => {
                    out.reset = Some((at, kind));
                    break;
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cfg(actions: [Action; 4]) -> Timing {
        Timing {
            enabled: true,
            actions,
            holds_ps: [100, 200, 300, 400],
        }
    }

    #[test]
    fn stages_run_in_order_and_feeding_restarts() {
        let c = cfg([
            Action::Interrupt,
            Action::Off,
            Action::Reset(ResetKind::Cpu),
            Action::Off,
        ]);
        let mut s = Stages::default();
        assert_eq!(s.next(&c), Some(SimTime(100)));
        assert_eq!(s.advance(SimTime(99), &c), Outcome::default());
        let o = s.advance(SimTime(250), &c);
        assert!(o.interrupt && o.reset.is_none());
        assert_eq!(s.stage(), 1);
        s.restart(SimTime(250));
        assert_eq!(s.next(&c), Some(SimTime(350)));
        let o = s.advance(SimTime(10_000), &c);
        assert_eq!(o.reset, Some((SimTime(350 + 200 + 300), ResetKind::Cpu)));
        assert_eq!(s.stage(), 3, "stops after the reset stage");
    }

    #[test]
    fn disabled_or_all_off_never_expires() {
        let mut c = cfg([Action::Off; 4]);
        assert_eq!(Stages::default().next(&c), None);
        c.actions[3] = Action::Interrupt;
        c.enabled = false;
        assert_eq!(Stages::default().next(&c), None);
    }
}
