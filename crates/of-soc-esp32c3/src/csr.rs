//! ESP32-C3 CSRs outside the standard machine set, served through
//! [`of_cpu_riscv::Bus::csr_read`] (TRM chapter "ESP-RISC-V CPU").
//!
//! - Performance counters: machine `mpcer`/`mpcmr`/`mpccr` (`0x7e0` to
//!   `0x7e2`) and the user-mode set `upcer`/`upcmr`/`upccr` (`0x800` to
//!   `0x802`). Only the cycle event is modelled: while `pcer` bit 0 (cycles)
//!   and `pcmr` bit 0 (count enable) are set, `pccr` advances with the
//!   hart's cycles; writing `pccr` sets it. The ROM's `ets_delay_us` spins
//!   on `upccr`, and ESP-IDF's cycle count reads `mpccr`.
//! - User-level trap CSRs (`ustatus`, `uie`, `utvec`, `uscratch`, `uepc`,
//!   `ucause`, `utval`, `uip`): stored, with no effect (U-mode is not
//!   modelled); the ROM clears `ustatus` at reset.
//!
//! Any other unknown CSR stays illegal and is counted in
//! [`C3Csrs::unknown`].

use std::collections::BTreeMap;

/// One performance counter set (enable, mode, count).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct PerfCounter {
    pcer: u32,
    pcmr: u32,
    /// Count at `since`.
    base: u32,
    since: u64,
}

impl PerfCounter {
    fn counting(&self) -> bool {
        self.pcer & 1 != 0 && self.pcmr & 1 != 0
    }

    fn count(&self, cycles: u64) -> u32 {
        if self.counting() {
            self.base.wrapping_add(cycles.saturating_sub(self.since) as u32)
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

const USER_CSRS: [u16; 8] = [0x000, 0x004, 0x005, 0x040, 0x041, 0x042, 0x043, 0x044];

/// The C3's extra CSRs.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct C3Csrs {
    perf: [PerfCounter; 2],
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

    fn perf_index(csr: u16) -> Option<(usize, u16)> {
        match csr {
            0x7e0..=0x7e2 => Some((0, csr - 0x7e0)),
            0x800..=0x802 => Some((1, csr - 0x800)),
            _ => None,
        }
    }

    /// Read a CSR; `None` if it does not exist.
    pub fn read(&mut self, csr: u16) -> Option<u32> {
        if let Some((i, reg)) = Self::perf_index(csr) {
            let p = &self.perf[i];
            return Some(match reg {
                0 => p.pcer,
                1 => p.pcmr,
                _ => p.count(self.cycles),
            });
        }
        if let Some(i) = USER_CSRS.iter().position(|&c| c == csr) {
            return Some(self.user[i]);
        }
        *self.unknown.entry(csr).or_default() += 1;
        None
    }

    /// Write a CSR; `false` if it does not exist.
    pub fn write(&mut self, csr: u16, v: u32) -> bool {
        let cycles = self.cycles;
        if let Some((i, reg)) = Self::perf_index(csr) {
            let p = &mut self.perf[i];
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
        assert!(c.write(0x800, 1) && c.write(0x801, 1));
        c.set_cycles(1_100);
        assert_eq!(c.read(0x802), Some(1_000));
        assert_eq!(c.read(0x7e2), Some(0), "machine counter is separate");
        assert!(c.write(0x801, 0));
        c.set_cycles(5_000);
        assert_eq!(c.read(0x802), Some(1_000), "stopped");
        assert!(c.write(0x802, 7) && c.write(0x801, 1));
        c.set_cycles(5_010);
        assert_eq!(c.read(0x802), Some(17));
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
