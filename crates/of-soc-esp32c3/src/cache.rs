//! The external memory cache and its MMU (TRM chapter "System and Memory",
//! "Cache"; register offsets as in ESP-IDF's `extmem_reg.h` and
//! `ext_mem_defs.h`).
//!
//! The ESP32-C3 has one 16 KiB instruction cache that serves both flash
//! windows: IBus at `0x4200_0000` (IROM, code) and DBus at `0x3c00_0000`
//! (DROM, read-only data), 8 MiB each. One MMU table of 128 entries at
//! `0x600c_5000` maps 64 KiB virtual pages to 64 KiB flash pages and is
//! shared by both buses: entry `n` translates offset `n * 64 KiB` of either
//! window. An entry holds the flash page number in bits 7:0; bit 8 set
//! marks it invalid.
//!
//! Simplifications, all deliberate:
//! - No cache lines are modelled. Every access through a window reads the
//!   flash straight through the MMU, so it always "hits", never sees stale
//!   data after a flash write, and costs no extra cycles (miss timing is a
//!   calibration item).
//! - Cache maintenance (invalidate, preload, autoload, lock, freeze) is
//!   accepted and reports done at once.
//! - An access with the cache disabled, the bus shut off or an invalid MMU
//!   entry is refused (the hart takes an access fault). On hardware the
//!   cache raises its error interrupt and returns garbage instead.

use crate::map;

/// Register offsets in the EXTMEM block (`0x600c_4000`).
pub mod reg {
    /// ICACHE_CTRL: bit 0 enables the cache.
    pub const ICACHE_CTRL: u32 = 0x000;
    /// ICACHE_CTRL1: bit 0 shuts IBus, bit 1 shuts DBus.
    pub const ICACHE_CTRL1: u32 = 0x004;
    /// Lock control (lock/unlock enable, done).
    pub const ICACHE_LOCK_CTRL: u32 = 0x01c;
    /// Invalidate control (enable, done).
    pub const ICACHE_SYNC_CTRL: u32 = 0x028;
    /// Preload control (enable, done).
    pub const ICACHE_PRELOAD_CTRL: u32 = 0x034;
    /// Autoload control (enable, done).
    pub const ICACHE_AUTOLOAD_CTRL: u32 = 0x040;
    /// Cache state (idle = 1).
    pub const CACHE_STATE: u32 = 0x0b0;
    /// Freeze control (enable, mode, done).
    pub const ICACHE_FREEZE: u32 = 0x0cc;
    /// Version register.
    pub const DATE: u32 = 0x3fc;
}

/// Number of MMU entries.
pub const MMU_ENTRIES: usize = 128;
/// MMU page size.
pub const PAGE: u32 = 64 << 10;
/// MMU entry bit 8: invalid.
pub const MMU_INVALID: u32 = 1 << 8;
/// MMU entry bits 7:0: flash page.
pub const MMU_PAGE_MASK: u32 = 0xff;

/// Which bus an access comes through.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheBus {
    /// Instruction bus, the IROM window at `0x4200_0000`.
    IBus,
    /// Data bus, the DROM window at `0x3c00_0000`.
    DBus,
}

/// Why a window access was refused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CacheFault {
    /// `ICACHE_CTRL.ENABLE` is 0.
    Disabled,
    /// The bus is shut off in `ICACHE_CTRL1`.
    BusShut,
    /// The MMU entry is marked invalid.
    InvalidEntry(usize),
}

const REGS: usize = 0x400 / 4;

/// The cache controller registers and the MMU table.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Cache {
    regs: Vec<u32>,
    mmu: [u32; MMU_ENTRIES],
}

impl Default for Cache {
    fn default() -> Self {
        Self::new()
    }
}

impl Cache {
    /// Reset state: cache disabled, both buses shut, every MMU entry
    /// invalid.
    pub fn new() -> Self {
        let mut regs = vec![0; REGS];
        regs[(reg::ICACHE_CTRL1 / 4) as usize] = 0b11;
        regs[(reg::CACHE_STATE / 4) as usize] = 1;
        // Nothing in progress: every operation reports done.
        regs[(reg::ICACHE_SYNC_CTRL / 4) as usize] = 0b10;
        regs[(reg::ICACHE_PRELOAD_CTRL / 4) as usize] = 0b10;
        regs[(reg::ICACHE_LOCK_CTRL / 4) as usize] = 0b100;
        regs[(reg::ICACHE_AUTOLOAD_CTRL / 4) as usize] = 0b1000;
        regs[(reg::DATE / 4) as usize] = 0x0200_4100;
        Self {
            regs,
            mmu: [MMU_INVALID; MMU_ENTRIES],
        }
    }

    fn r(&self, off: u32) -> u32 {
        self.regs[(off / 4) as usize]
    }

    /// Read an EXTMEM register.
    pub fn read(&self, off: u32) -> Option<u32> {
        self.regs.get((off / 4) as usize).copied()
    }

    /// Write an EXTMEM register. Maintenance operations complete at once.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        let old = *self.regs.get((off / 4) as usize)?;
        // (enable bits, done bit) per operation register.
        let (ena, done) = match off {
            reg::ICACHE_SYNC_CTRL | reg::ICACHE_PRELOAD_CTRL => (0b1, 0b10),
            reg::ICACHE_LOCK_CTRL => (0b11, 0b100),
            reg::ICACHE_AUTOLOAD_CTRL => (0, 0b1000),
            reg::ICACHE_FREEZE => {
                self.regs[(off / 4) as usize] = (v & 0b11) | if v & 1 != 0 { 0b100 } else { 0 };
                return Some(());
            }
            reg::CACHE_STATE | reg::DATE => return Some(()),
            _ => (0, 0),
        };
        let new = if done == 0 {
            v
        } else if v & ena != 0 || ena == 0 {
            (v & !ena & !done) | done
        } else {
            (v & !done) | (old & done)
        };
        self.regs[(off / 4) as usize] = new;
        Some(())
    }

    /// Read an MMU table word (offset within the `0x600c_5000` block).
    pub fn mmu_read(&self, off: u32) -> Option<u32> {
        self.mmu.get((off / 4) as usize).copied()
    }

    /// Write an MMU table word.
    pub fn mmu_write(&mut self, off: u32, v: u32) -> Option<()> {
        let e = self.mmu.get_mut((off / 4) as usize)?;
        *e = v & (MMU_INVALID | MMU_PAGE_MASK);
        Some(())
    }

    /// MMU entry `n`.
    pub fn entry(&self, n: usize) -> u32 {
        self.mmu[n]
    }

    /// Set MMU entry `n` to flash page `page` (valid).
    pub fn map(&mut self, n: usize, page: u32) {
        self.mmu[n] = page & MMU_PAGE_MASK;
    }

    /// Enable the cache and open both buses, as a bootloader does before
    /// jumping to an app in flash.
    pub fn enable(&mut self) {
        self.regs[(reg::ICACHE_CTRL / 4) as usize] |= 1;
        self.regs[(reg::ICACHE_CTRL1 / 4) as usize] &= !0b11;
    }

    /// Translate a window address to a flash byte address.
    pub fn translate(&self, addr: u32, bus: CacheBus) -> Result<u32, CacheFault> {
        if self.r(reg::ICACHE_CTRL) & 1 == 0 {
            return Err(CacheFault::Disabled);
        }
        let shut = match bus {
            CacheBus::IBus => 0b01,
            CacheBus::DBus => 0b10,
        };
        if self.r(reg::ICACHE_CTRL1) & shut != 0 {
            return Err(CacheFault::BusShut);
        }
        let linear = addr & (map::IROM_FLASH.1 - 1);
        let n = (linear / PAGE) as usize;
        let e = self.mmu[n];
        if e & MMU_INVALID != 0 {
            return Err(CacheFault::InvalidEntry(n));
        }
        Ok((e & MMU_PAGE_MASK) * PAGE + linear % PAGE)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn translation_needs_enable_open_bus_and_valid_entry() {
        let mut c = Cache::new();
        assert_eq!(c.translate(0x4200_0000, CacheBus::IBus), Err(CacheFault::Disabled));
        c.write(reg::ICACHE_CTRL, 1).unwrap();
        assert_eq!(c.translate(0x4200_0000, CacheBus::IBus), Err(CacheFault::BusShut));
        c.write(reg::ICACHE_CTRL1, 0b10).unwrap();
        assert_eq!(
            c.translate(0x4200_0000, CacheBus::IBus),
            Err(CacheFault::InvalidEntry(0))
        );
        assert_eq!(c.translate(0x3c00_0000, CacheBus::DBus), Err(CacheFault::BusShut));
        c.mmu_write(4 * 3, 5).unwrap();
        assert_eq!(c.translate(0x4203_1234, CacheBus::IBus), Ok(0x5_1234));
        c.write(reg::ICACHE_CTRL1, 0).unwrap();
        assert_eq!(
            c.translate(0x3c03_fffc, CacheBus::DBus),
            Ok(0x5_fffc),
            "one table for both buses"
        );
        c.mmu_write(4 * 3, MMU_INVALID | 5).unwrap();
        assert_eq!(
            c.translate(0x3c03_0000, CacheBus::DBus),
            Err(CacheFault::InvalidEntry(3))
        );
        assert_eq!(c.mmu_read(4 * 3), Some(0x105));
        assert_eq!(c.mmu_read(0x200), None, "128 entries");
    }

    #[test]
    fn maintenance_operations_complete_at_once() {
        let mut c = Cache::new();
        assert_eq!(c.read(reg::ICACHE_AUTOLOAD_CTRL), Some(0b1000), "idle at reset");
        c.write(reg::ICACHE_SYNC_CTRL, 1).unwrap();
        assert_eq!(c.read(reg::ICACHE_SYNC_CTRL), Some(0b10), "invalidate done");
        c.write(reg::ICACHE_LOCK_CTRL, 0b01).unwrap();
        assert_eq!(c.read(reg::ICACHE_LOCK_CTRL), Some(0b100));
        c.write(reg::ICACHE_FREEZE, 0b11).unwrap();
        assert_eq!(c.read(reg::ICACHE_FREEZE), Some(0b111));
        c.write(reg::ICACHE_FREEZE, 0).unwrap();
        assert_eq!(c.read(reg::ICACHE_FREEZE), Some(0));
        assert_eq!(c.read(reg::CACHE_STATE), Some(1), "idle");
        c.write(reg::ICACHE_AUTOLOAD_CTRL, 0b100).unwrap();
        assert_eq!(c.read(reg::ICACHE_AUTOLOAD_CTRL), Some(0b1100));
    }
}
