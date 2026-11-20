//! The memory/bus interface between the hart and the SoC.
//!
//! The CPU core knows nothing about the ESP32-C3 memory map. The SoC crate
//! implements [`Bus`] to route accesses to ROM, flash cache, IRAM, DRAM and
//! peripheral registers, and to add wait states and custom CSRs.

use of_core::SimTime;

/// Width of a memory access.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Size {
    /// 8 bits.
    Byte,
    /// 16 bits.
    Half,
    /// 32 bits.
    Word,
}

impl Size {
    /// Width in bytes.
    pub const fn bytes(self) -> u32 {
        match self {
            Size::Byte => 1,
            Size::Half => 2,
            Size::Word => 4,
        }
    }
}

/// Why the hart is accessing memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AccessKind {
    /// Instruction fetch.
    Fetch,
    /// Data load.
    Load,
    /// Data store.
    Store,
    /// Read or write half of an atomic memory operation.
    Atomic,
}

/// A failed access: no device at the address, or the device refused it.
/// The hart turns this into an access-fault exception.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BusError {
    /// Faulting address.
    pub addr: u32,
}

/// Memory and side-band services the SoC provides to the hart.
pub trait Bus {
    /// Read `size` bytes at `addr` (little-endian), zero-extended to 32 bits.
    fn read(&mut self, addr: u32, size: Size, kind: AccessKind) -> Result<u32, BusError>;

    /// Write the low `size` bytes of `value` at `addr` (little-endian).
    fn write(&mut self, addr: u32, size: Size, value: u32, kind: AccessKind) -> Result<(), BusError>;

    /// Extra cycles the last access at `addr` costs (flash cache misses,
    /// peripheral bus latency). Added to the instruction's cycle count.
    fn wait_states(&self, _addr: u32, _kind: AccessKind) -> u32 {
        0
    }

    /// Called with the hart's elapsed cycle count right before
    /// [`Self::csr_read`] or [`Self::csr_write`], so SoC CSRs that count
    /// cycles (such as the ESP32-C3 performance counter) can see the time.
    fn csr_cycles(&mut self, _cycles: u64) {}

    /// Read a CSR the core does not implement (for example ESP32-C3 custom
    /// CSRs). `None` makes the instruction illegal.
    fn csr_read(&mut self, _csr: u16) -> Option<u32> {
        None
    }

    /// Write a CSR the core does not implement. Return `false` to make the
    /// instruction illegal.
    fn csr_write(&mut self, _csr: u16, _value: u32) -> bool {
        false
    }

    /// Called before a data load (or the read half of an atomic) from
    /// `addr`, with the hart's local time at the start of the instruction.
    /// Return `true` to stop *before* the instruction: it does not execute
    /// (registers, memory and time are unchanged) and [`crate::Hart::run`]
    /// returns [`crate::RunExit::Deferred`]. A SoC uses this when the value
    /// depends on engines that have not reached `now` yet, such as GPIO
    /// inputs fed by the analog solver. It must return `false` once they
    /// have, or the hart makes no progress.
    fn defer_load(&mut self, _addr: u32, _now: SimTime) -> bool {
        false
    }

    /// Return `true` (once) after an access that needs the rest of the
    /// simulation to catch up, typically any peripheral register access. The
    /// run loop then stops after the current instruction so the scheduler can
    /// advance the analog and digital engines to the hart's local time.
    fn take_sync_request(&mut self) -> bool {
        false
    }
}

/// A flat, little-endian RAM region. Useful for tests and as a building block
/// for SoC memory maps.
#[derive(Debug, Clone)]
pub struct Ram {
    base: u32,
    data: Vec<u8>,
}

impl Ram {
    /// Zero-filled RAM of `len` bytes starting at `base`.
    pub fn new(base: u32, len: usize) -> Self {
        Self {
            base,
            data: vec![0; len],
        }
    }

    /// Base address.
    pub fn base(&self) -> u32 {
        self.base
    }

    /// Whether `[addr, addr + len)` lies inside this region.
    pub fn contains(&self, addr: u32, len: u32) -> bool {
        let off = u64::from(addr.wrapping_sub(self.base));
        addr >= self.base && off + u64::from(len) <= self.data.len() as u64
    }

    /// Copy `bytes` into RAM at `addr`.
    pub fn load(&mut self, addr: u32, bytes: &[u8]) -> Result<(), BusError> {
        if !self.contains(addr, bytes.len() as u32) {
            return Err(BusError { addr });
        }
        let off = (addr - self.base) as usize;
        self.data[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Store 32-bit words (for example hand-assembled code) at `addr`.
    pub fn load_words(&mut self, addr: u32, words: &[u32]) -> Result<(), BusError> {
        let bytes: Vec<u8> = words.iter().flat_map(|w| w.to_le_bytes()).collect();
        self.load(addr, &bytes)
    }

    /// Borrow the bytes at `[addr, addr + len)`.
    pub fn slice(&self, addr: u32, len: usize) -> Option<&[u8]> {
        if !self.contains(addr, len as u32) {
            return None;
        }
        let off = (addr - self.base) as usize;
        Some(&self.data[off..off + len])
    }

    /// Read without side effects (debugger use).
    pub fn peek(&self, addr: u32, size: Size) -> Option<u32> {
        let b = self.slice(addr, size.bytes() as usize)?;
        Some(b.iter().rev().fold(0u32, |acc, &x| (acc << 8) | u32::from(x)))
    }
}

impl Bus for Ram {
    fn read(&mut self, addr: u32, size: Size, _kind: AccessKind) -> Result<u32, BusError> {
        self.peek(addr, size).ok_or(BusError { addr })
    }

    fn write(&mut self, addr: u32, size: Size, value: u32, _kind: AccessKind) -> Result<(), BusError> {
        let n = size.bytes() as usize;
        self.load(addr, &value.to_le_bytes()[..n])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ram_is_little_endian_and_bounds_checked() {
        let mut ram = Ram::new(0x1000, 8);
        ram.write(0x1000, Size::Word, 0x1122_3344, AccessKind::Store).unwrap();
        assert_eq!(ram.read(0x1000, Size::Byte, AccessKind::Load), Ok(0x44));
        assert_eq!(ram.read(0x1002, Size::Half, AccessKind::Load), Ok(0x1122));
        assert_eq!(
            ram.read(0x1006, Size::Word, AccessKind::Load),
            Err(BusError { addr: 0x1006 })
        );
        assert_eq!(
            ram.read(0x0fff, Size::Byte, AccessKind::Load),
            Err(BusError { addr: 0x0fff })
        );
    }
}
