//! ESP32-C3 CSRs outside the standard machine set, served through
//! [`of_cpu_riscv::Bus::csr_read`] (TRM chapter "ESP-RISC-V CPU").
//!
//! - Performance counter: one counter, seen as the machine CSRs
//!   `mpcer`/`mpcmr`/`mpccr` (`0x7e0` to `0x7e2`) and as `upcer`/`upcmr`/
//!   `upccr` (`0x800` to `0x802`), the names ESP-IDF uses from user mode.
//!   They are one counter, not two: the ROM enables counting through
//!   `upcer`/`upcmr` only and its `ets_delay_us` spins on `upccr`, while the
//!   ESP-IDF bootloader never touches `mpcer` and waits on `mpccr` (in
//!   `bootloader_fill_random`), which only works on hardware if both names
//!   reach the same counter. Only the cycle event is modelled: while `pcer`
//!   bit 0 (cycles) and `pcmr` bit 0 (count enable) are set, `pccr`
//!   advances with the hart's cycles; writing `pccr` sets it. `pcmr` resets
//!   to 3 (TRM: `COUNT_EN` and `COUNT_SAT` set), so setting the cycle event
//!   alone starts the count, and with `COUNT_SAT` the count halts at the
//!   maximum instead of wrapping. Cycles in `wfi` are counted (the TRM says
//!   they are not; the hart does not report them separately).
//! - User-level trap CSRs (`ustatus`, `uie`, `utvec`, `uscratch`, `uepc`,
//!   `ucause`, `utval`, `uip`): stored, with no effect (U-mode is not
//!   modelled); the ROM clears `ustatus` at reset.
//! - Physical memory protection: `pmpcfg0` to `pmpcfg3` (`0x3a0` to `0x3a3`)
//!   and `pmpaddr0` to `pmpaddr15` (`0x3b0` to `0x3bf`), 16 entries. They
//!   are stored with the lock rule (an entry whose `L` bit is set, or whose
//!   address is the top of a locked TOR entry, ignores writes until reset),
//!   but **not enforced**: accesses are never checked against them. The
//!   ESP-IDF bootloader and esp-hal set them up at start.
//! - Debug triggers: `tselect` (`0x7a0`, 8 triggers), `tdata1`/`tdata2`
//!   (`0x7a1`/`0x7a2`, per trigger) and `tcontrol` (`0x7a5`, `MPTE` and
//!   `MTE`). `tdata1` always reads as an address-match trigger (`type` 2 in
//!   bits 31:28, `dmode` 0). Stored, **not enforced**: no breakpoint or
//!   watchpoint ever fires (esp-hal arms one as a stack guard).
//!
//! Any other unknown CSR stays illegal and is counted in
//! [`C3Csrs::unknown`].

use std::collections::BTreeMap;

/// One performance counter set (enable, mode, count).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PerfCounter {
    pcer: u32,
    pcmr: u32,
    /// Count at `since`.
    base: u32,
    since: u64,
}

impl Default for PerfCounter {
    fn default() -> Self {
        Self {
            pcer: 0,
            pcmr: PCMR_RESET,
            base: 0,
            since: 0,
        }
    }
}

/// `pcmr` at reset: `COUNT_SAT` (bit 1) and `COUNT_EN` (bit 0).
const PCMR_RESET: u32 = 3;

impl PerfCounter {
    fn counting(&self) -> bool {
        self.pcer & 1 != 0 && self.pcmr & 1 != 0
    }

    fn count(&self, cycles: u64) -> u32 {
        if self.counting() {
            let delta = cycles.saturating_sub(self.since);
            if self.pcmr & 2 != 0 {
                (u64::from(self.base) + delta).min(u64::from(u32::MAX)) as u32
            } else {
                self.base.wrapping_add(delta as u32)
            }
        } else {
            self.base
        }
    }

    /// Re-base at `cycles` so a mode change starts from the current count.
    fn rebase(&mut self, cycles: u64) {
        self.base = self.count(cycles);
        self.since = cycles;
    }
}

/// PMP entries on the C3.
const PMP_ENTRIES: usize = 16;
/// `pmpcfg` lock bit.
const PMP_L: u8 = 0x80;
/// `pmpcfg` address-matching field, `TOR` value.
const PMP_A_MASK: u8 = 0x18;
const PMP_A_TOR: u8 = 0x08;

/// Debug triggers on the C3.
const TRIGGERS: usize = 8;
/// `tdata1.type` = 2 (address/data match), `dmode` = 0.
const TDATA1_TYPE: u32 = 2 << 28;
const TDATA1_FIXED: u32 = 0xf << 28 | 1 << 27;
/// `tcontrol` writable bits: `MPTE` (7) and `MTE` (3).
const TCONTROL_MASK: u32 = 0x88;

const USER_CSRS: [u16; 8] = [0x000, 0x004, 0x005, 0x040, 0x041, 0x042, 0x043, 0x044];

/// The C3's extra CSRs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct C3Csrs {
    perf: PerfCounter,
    pmpcfg: [u8; PMP_ENTRIES],
    pmpaddr: [u32; PMP_ENTRIES],
    tselect: u32,
    tdata1: [u32; TRIGGERS],
    tdata2: [u32; TRIGGERS],
    tcontrol: u32,
    user: [u32; USER_CSRS.len()],
    cycles: u64,
    /// Accesses to CSRs that are not modelled, by number, with counts.
    pub unknown: BTreeMap<u16, u64>,
}

impl C3Csrs {
    /// Reset state.
    pub fn new() -> Self {
        Self::default()
    }

    /// The hart's elapsed cycles at the coming access.
    pub fn set_cycles(&mut self, cycles: u64) {
        self.cycles = cycles;
    }

    /// Which performance counter register `csr` names (0 `pcer`, 1
    /// `pcmr`, 2 `pccr`), under either name.
    fn perf_reg(csr: u16) -> Option<u16> {
        match csr {
            0x7e0..=0x7e2 => Some(csr - 0x7e0),
            0x800..=0x802 => Some(csr - 0x800),
            _ => None,
        }
    }

    /// Read a CSR; `None` if it does not exist.
    pub fn read(&mut self, csr: u16) -> Option<u32> {
        if let Some(reg) = Self::perf_reg(csr) {
            let p = &self.perf;
            return Some(match reg {
                0 => p.pcer,
                1 => p.pcmr,
                _ => p.count(self.cycles),
            });
        }
        if let Some(i) = USER_CSRS.iter().position(|&c| c == csr) {
            return Some(self.user[i]);
        }
        match csr {
            0x3a0..=0x3a3 => {
                let i = usize::from(csr - 0x3a0) * 4;
                return Some(u32::from_le_bytes(self.pmpcfg[i..i + 4].try_into().unwrap_or([0; 4])));
            }
            0x3b0..=0x3bf => return Some(self.pmpaddr[usize::from(csr - 0x3b0)]),
            0x7a0 => return Some(self.tselect),
            0x7a1 => return Some(self.tdata1[self.tselect as usize] & !TDATA1_FIXED | TDATA1_TYPE),
            0x7a2 => return Some(self.tdata2[self.tselect as usize]),
            0x7a5 => return Some(self.tcontrol),
            _ => {}
        }
        *self.unknown.entry(csr).or_default() += 1;
        None
    }

    /// Whether `pmpaddr[i]` ignores writes: its entry is locked, or the next
    /// entry is a locked TOR range that uses it as its bottom.
    fn pmpaddr_locked(&self, i: usize) -> bool {
        let next_tor_locked = self
            .pmpcfg
            .get(i + 1)
            .is_some_and(|&c| c & PMP_L != 0 && c & PMP_A_MASK == PMP_A_TOR);
        self.pmpcfg[i] & PMP_L != 0 || next_tor_locked
    }

    /// Write a CSR; `false` if it does not exist.
    pub fn write(&mut self, csr: u16, v: u32) -> bool {
        let cycles = self.cycles;
        if let Some(reg) = Self::perf_reg(csr) {
            let p = &mut self.perf;
            p.rebase(cycles);
            match reg {
                0 => p.pcer = v,
                1 => p.pcmr = v,
                _ => p.base = v,
            }
            return true;
        }
        if let Some(i) = USER_CSRS.iter().position(|&c| c == csr) {
            self.user[i] = v;
            return true;
        }
        match csr {
            0x3a0..=0x3a3 => {
                let base = usize::from(csr - 0x3a0) * 4;
                for (k, b) in v.to_le_bytes().into_iter().enumerate() {
                    if self.pmpcfg[base + k] & PMP_L == 0 {
                        self.pmpcfg[base + k] = b;
                    }
                }
                return true;
            }
            0x3b0..=0x3bf => {
                let i = usize::from(csr - 0x3b0);
                if !self.pmpaddr_locked(i) {
                    self.pmpaddr[i] = v;
                }
                return true;
            }
            0x7a0 => {
                // WARL: an out-of-range trigger number is not stored.
                if (v as usize) < TRIGGERS {
                    self.tselect = v;
                }
                return true;
            }
            0x7a1 => {
                self.tdata1[self.tselect as usize] = v & !TDATA1_FIXED | TDATA1_TYPE;
                return true;
            }
            0x7a2 => {
                self.tdata2[self.tselect as usize] = v;
                return true;
            }
            0x7a5 => {
                self.tcontrol = v & TCONTROL_MASK;
                return true;
            }
            _ => {}
        }
        *self.unknown.entry(csr).or_default() += 1;
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn performance_counter_counts_cycles_while_enabled() {
        let mut c = C3Csrs::new();
        c.set_cycles(100);
        assert_eq!(c.read(0x802), Some(0), "off at reset");
        assert_eq!(c.read(0x801), Some(3), "COUNT_EN and COUNT_SAT at reset");
        assert_eq!(c.read(0x7e1), Some(3));
        // The cycle event alone starts the count.
        assert!(c.write(0x800, 1));
        c.set_cycles(1_100);
        assert_eq!(c.read(0x802), Some(1_000));
        assert_eq!(c.read(0x7e2), Some(1_000), "one counter under both names");
        assert_eq!(c.read(0x7e0), Some(1));
        assert!(c.write(0x801, 0));
        c.set_cycles(5_000);
        assert_eq!(c.read(0x802), Some(1_000), "stopped");
        assert!(c.write(0x802, 7) && c.write(0x801, 1));
        c.set_cycles(5_010);
        assert_eq!(c.read(0x802), Some(17));
    }

    #[test]
    fn pmp_csrs_are_stored_and_locked_entries_ignore_writes() {
        let mut c = C3Csrs::new();
        assert!(c.write(0x3b0, 0x1000_0000) && c.write(0x3b1, 0x2000_0000));
        // Entry 1: TOR, locked, RWX; entry 0 unlocked, off.
        assert!(c.write(0x3a0, 0x8f << 8));
        assert_eq!(c.read(0x3a0), Some(0x8f << 8));
        assert!(
            c.write(0x3b0, 1) && c.write(0x3b1, 2),
            "writes to locked entries are ignored, not illegal"
        );
        assert_eq!(c.read(0x3b0), Some(0x1000_0000), "bottom of a locked TOR range");
        assert_eq!(c.read(0x3b1), Some(0x2000_0000));
        assert!(c.write(0x3a0, 0x07));
        assert_eq!(c.read(0x3a0), Some(0x8f07), "only the unlocked byte changed");
        assert!(c.write(0x3bf, 5) && c.write(0x3a3, 0x1f << 24));
        assert_eq!((c.read(0x3bf), c.read(0x3a3)), (Some(5), Some(0x1f << 24)));
        assert!(c.unknown.is_empty());
    }

    #[test]
    fn performance_counter_saturates_or_wraps() {
        let mut c = C3Csrs::new();
        assert!(c.write(0x7e2, u32::MAX - 5) && c.write(0x7e0, 1));
        c.set_cycles(100);
        assert_eq!(c.read(0x7e2), Some(u32::MAX), "COUNT_SAT halts at the maximum");
        assert!(c.write(0x7e2, u32::MAX - 5) && c.write(0x7e1, 1));
        c.set_cycles(110);
        assert_eq!(c.read(0x7e2), Some(4), "wraps without COUNT_SAT");
    }

    #[test]
    fn trigger_csrs_are_stored_per_trigger() {
        let mut c = C3Csrs::new();
        assert_eq!(c.read(0x7a1), Some(2 << 28), "address-match trigger");
        assert!(c.write(0x7a0, 1) && c.write(0x7a1, 0x0800_0047) && c.write(0x7a2, 0x3fc8_0000));
        assert!(c.write(0x7a0, 9), "out of range: ignored");
        assert_eq!(c.read(0x7a0), Some(1));
        assert_eq!(c.read(0x7a1), Some(0x2000_0047), "type fixed, dmode 0");
        assert_eq!(c.read(0x7a2), Some(0x3fc8_0000));
        assert!(c.write(0x7a0, 0));
        assert_eq!(c.read(0x7a2), Some(0));
        assert!(c.write(0x7a5, 0xff));
        assert_eq!(c.read(0x7a5), Some(0x88));
        assert!(c.unknown.is_empty());
    }

    #[test]
    fn user_csrs_are_stored_and_others_are_illegal() {
        let mut c = C3Csrs::new();
        assert!(c.write(0x000, 1));
        assert_eq!(c.read(0x000), Some(1));
        assert_eq!(c.read(0x7c0), None);
        assert!(!c.write(0x7c0, 1));
        assert_eq!(c.unknown.get(&0x7c0), Some(&2));
    }
}
