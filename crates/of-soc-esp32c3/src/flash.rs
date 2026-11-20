//! The external SPI NOR flash chip behind SPI0/SPI1.
//!
//! A byte array (erased bytes read `0xFF`) plus the parts of a standard
//! SPI NOR command set that the ROM, bootloaders and flash drivers use:
//! JEDEC ID, status registers, write enable, page program (which can only
//! clear bits, wrapping inside the 256-byte page), 4 KiB sector, 32/64 KiB
//! block and chip erase. Commands complete instantly: the busy bit (WIP)
//! always reads 0 and no program or erase time is modelled.

/// Default capacity: 4 MiB, the usual size on ESP32-C3 modules.
pub const DEFAULT_SIZE: usize = 4 << 20;
/// Erase granularity of the sector erase command (0x20).
pub const SECTOR: usize = 4096;
/// Page program wraps inside one 256-byte page.
pub const PAGE: usize = 256;

/// Status register bit 0: write in progress (always 0 here).
pub const SR_WIP: u16 = 1 << 0;
/// Status register bit 1: write enable latch.
pub const SR_WEL: u16 = 1 << 1;

/// Standard SPI NOR command bytes understood by [`SpiFlash::command`].
pub mod cmd {
    /// Write status register 1 (and 2 with a 16-bit write).
    pub const WRSR: u8 = 0x01;
    /// Page program.
    pub const PP: u8 = 0x02;
    /// Read data.
    pub const READ: u8 = 0x03;
    /// Write disable.
    pub const WRDI: u8 = 0x04;
    /// Read status register 1.
    pub const RDSR: u8 = 0x05;
    /// Write enable.
    pub const WREN: u8 = 0x06;
    /// Fast read (and the dual/quad variants below).
    pub const FAST_READ: u8 = 0x0b;
    /// Sector erase (4 KiB).
    pub const SE: u8 = 0x20;
    /// Write status register 2.
    pub const WRSR2: u8 = 0x31;
    /// Read status register 2.
    pub const RDSR2: u8 = 0x35;
    /// Dual output fast read.
    pub const DOUT_READ: u8 = 0x3b;
    /// Write enable for volatile status register.
    pub const VSR_WREN: u8 = 0x50;
    /// Block erase (32 KiB).
    pub const BE32: u8 = 0x52;
    /// Read SFDP.
    pub const RDSFDP: u8 = 0x5a;
    /// Chip erase.
    pub const CE: u8 = 0x60;
    /// Quad output fast read.
    pub const QOUT_READ: u8 = 0x6b;
    /// Read JEDEC ID.
    pub const RDID: u8 = 0x9f;
    /// Release from deep power-down.
    pub const RES: u8 = 0xab;
    /// Deep power-down.
    pub const DP: u8 = 0xb9;
    /// Dual I/O fast read.
    pub const DIO_READ: u8 = 0xbb;
    /// Chip erase (alternative opcode).
    pub const CE2: u8 = 0xc7;
    /// Block erase (64 KiB).
    pub const BE: u8 = 0xd8;
    /// Page erase (256 bytes; on some parts only).
    pub const PE: u8 = 0x81;
    /// Quad I/O fast read.
    pub const QIO_READ: u8 = 0xeb;
}

/// A SPI NOR flash chip.
#[derive(Clone, PartialEq, Eq)]
pub struct SpiFlash {
    data: Vec<u8>,
    jedec: [u8; 3],
    status: u16,
    /// Commands not understood, by opcode, with counts.
    unknown: Vec<(u8, u64)>,
}

impl std::fmt::Debug for SpiFlash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SpiFlash")
            .field("size", &self.data.len())
            .field("jedec", &self.jedec)
            .field("status", &self.status)
            .finish_non_exhaustive()
    }
}

impl Default for SpiFlash {
    fn default() -> Self {
        Self::new(DEFAULT_SIZE)
    }
}

impl SpiFlash {
    /// An erased chip of `size` bytes (rounded up to a power of two, at
    /// least 64 KiB, at most 16 MiB). The JEDEC ID is that of a GigaDevice
    /// GD25Q-series part of that size.
    pub fn new(size: usize) -> Self {
        let size = size.clamp(64 << 10, 16 << 20).next_power_of_two();
        Self {
            data: vec![0xff; size],
            jedec: [0xc8, 0x40, size.trailing_zeros() as u8],
            status: 0,
            unknown: Vec::new(),
        }
    }

    /// Capacity in bytes.
    pub fn size(&self) -> usize {
        self.data.len()
    }

    /// The three JEDEC ID bytes (manufacturer, memory type, capacity).
    pub fn jedec_id(&self) -> [u8; 3] {
        self.jedec
    }

    /// Status registers 1 (low byte) and 2 (high byte).
    pub fn status(&self) -> u16 {
        self.status
    }

    /// Back-door write (no erase needed), as a programmer would leave the
    /// chip. Bytes past the end wrap, like the chip's address counter.
    pub fn write_raw(&mut self, offset: u32, bytes: &[u8]) {
        for (i, b) in bytes.iter().enumerate() {
            let at = self.wrap(offset as usize + i);
            self.data[at] = *b;
        }
    }

    /// Read `buf.len()` bytes from `offset` (wrapping at the end).
    pub fn read(&self, offset: u32, buf: &mut [u8]) {
        for (i, b) in buf.iter_mut().enumerate() {
            *b = self.data[self.wrap(offset as usize + i)];
        }
    }

    /// Borrow the whole array.
    pub fn bytes(&self) -> &[u8] {
        &self.data
    }

    /// Byte at `offset` (wrapping).
    pub fn byte(&self, offset: u32) -> u8 {
        self.data[self.wrap(offset as usize)]
    }

    fn wrap(&self, at: usize) -> usize {
        at & (self.data.len() - 1)
    }

    /// Opcodes [`Self::command`] did not understand, with counts.
    pub fn unknown_commands(&self) -> &[(u8, u64)] {
        &self.unknown
    }

    fn take_wel(&mut self) -> bool {
        let wel = self.status & SR_WEL != 0;
        self.status &= !SR_WEL;
        wel
    }

    fn erase(&mut self, addr: u32, len: usize) {
        if self.take_wel() {
            let start = self.wrap(addr as usize) & !(len - 1);
            self.data[start..start + len].fill(0xff);
        }
    }

    /// Run one transaction: opcode `op`, `addr` (if the command has one),
    /// `mosi` bytes sent after it, and `miso.len()` bytes read back.
    pub fn command(&mut self, op: u8, addr: u32, mosi: &[u8], miso: &mut [u8]) {
        miso.fill(0xff);
        match op {
            cmd::READ | cmd::FAST_READ | cmd::DOUT_READ | cmd::QOUT_READ | cmd::DIO_READ | cmd::QIO_READ => {
                self.read(addr, miso)
            }
            cmd::RDID => {
                for (i, b) in miso.iter_mut().enumerate() {
                    *b = self.jedec.get(i).copied().unwrap_or(0);
                }
            }
            cmd::RDSR => miso.iter_mut().for_each(|b| *b = self.status as u8),
            cmd::RDSR2 => miso.iter_mut().for_each(|b| *b = (self.status >> 8) as u8),
            cmd::WREN => self.status |= SR_WEL,
            cmd::WRDI => self.status &= !SR_WEL,
            cmd::VSR_WREN | cmd::RES | cmd::DP | 0x66 | 0x99 | 0x75 | 0x7a | 0xb7 | 0xe9 | 0xff => {}
            cmd::WRSR | cmd::WRSR2 => {
                if self.take_wel() {
                    let keep = SR_WIP | SR_WEL;
                    let new = match (op, mosi) {
                        (cmd::WRSR, [lo, hi, ..]) => u16::from_le_bytes([*lo, *hi]),
                        (cmd::WRSR, [lo]) => (self.status & 0xff00) | u16::from(*lo),
                        (cmd::WRSR2, [hi, ..]) => (self.status & 0x00ff) | (u16::from(*hi) << 8),
                        _ => self.status,
                    };
                    self.status = new & !keep;
                }
            }
            cmd::PP => {
                if self.take_wel() {
                    let page = self.wrap(addr as usize) & !(PAGE - 1);
                    for (i, b) in mosi.iter().enumerate() {
                        let at = page + ((addr as usize + i) & (PAGE - 1));
                        self.data[at] &= *b;
                    }
                }
            }
            cmd::SE => self.erase(addr, SECTOR),
            cmd::PE => self.erase(addr, PAGE),
            cmd::BE32 => self.erase(addr, 32 << 10),
            cmd::BE => self.erase(addr, 64 << 10),
            cmd::CE | cmd::CE2 => {
                if self.take_wel() {
                    self.data.fill(0xff);
                }
            }
            cmd::RDSFDP => {}
            other => match self.unknown.iter_mut().find(|(o, _)| *o == other) {
                Some((_, n)) => *n += 1,
                None => self.unknown.push((other, 1)),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn erased_chip_reads_ff_and_reports_its_size() {
        let f = SpiFlash::new(4 << 20);
        assert_eq!(f.size(), 4 << 20);
        assert_eq!(f.jedec_id(), [0xc8, 0x40, 0x16]);
        let mut id = [0; 3];
        let mut f = f;
        f.command(cmd::RDID, 0, &[], &mut id);
        assert_eq!(id, [0xc8, 0x40, 0x16]);
        assert_eq!(f.byte(0x12345), 0xff);
        assert_eq!(SpiFlash::new(3 << 20).size(), 4 << 20, "rounded up");
    }

    #[test]
    fn program_needs_write_enable_and_only_clears_bits() {
        let mut f = SpiFlash::new(1 << 20);
        f.command(cmd::PP, 0x100, &[0x00], &mut []);
        assert_eq!(f.byte(0x100), 0xff, "no WREN, no program");
        f.command(cmd::WREN, 0, &[], &mut []);
        assert_eq!(f.status() & SR_WEL, SR_WEL);
        f.command(cmd::PP, 0x100, &[0x0f, 0xf0], &mut []);
        assert_eq!(f.status() & SR_WEL, 0, "WEL clears after the program");
        f.command(cmd::WREN, 0, &[], &mut []);
        f.command(cmd::PP, 0x100, &[0xf3], &mut []);
        assert_eq!([f.byte(0x100), f.byte(0x101)], [0x03, 0xf0], "AND, not overwrite");
        // A program running off the end of the page wraps to its start.
        f.command(cmd::WREN, 0, &[], &mut []);
        f.command(cmd::PP, 0x1ff, &[0x11, 0x22], &mut []);
        assert_eq!([f.byte(0x1ff), f.byte(0x100)], [0x11, 0x03 & 0x22]);
    }

    #[test]
    fn erase_commands_cover_aligned_blocks() {
        let mut f = SpiFlash::new(1 << 20);
        f.write_raw(0, &vec![0; 0x2_0000]);
        f.command(cmd::WREN, 0, &[], &mut []);
        f.command(cmd::SE, 0x1234, &[], &mut []);
        assert_eq!(
            (f.byte(0xfff), f.byte(0x1000), f.byte(0x1fff), f.byte(0x2000)),
            (0, 0xff, 0xff, 0)
        );
        f.command(cmd::WREN, 0, &[], &mut []);
        f.command(cmd::BE, 0x1_0005, &[], &mut []);
        assert_eq!((f.byte(0xffff), f.byte(0x1_0000), f.byte(0x1_ffff)), (0, 0xff, 0xff));
        f.command(cmd::WREN, 0, &[], &mut []);
        f.command(cmd::CE, 0, &[], &mut []);
        assert!(f.bytes().iter().all(|&b| b == 0xff));
    }

    #[test]
    fn reads_and_status_registers() {
        let mut f = SpiFlash::new(1 << 20);
        f.write_raw(0x10, b"hello");
        let mut buf = [0; 5];
        f.command(cmd::QIO_READ, 0x10, &[], &mut buf);
        assert_eq!(&buf, b"hello");
        f.command(cmd::WREN, 0, &[], &mut []);
        f.command(cmd::WRSR, 0, &[0x00, 0x02], &mut []);
        let mut sr2 = [0];
        f.command(cmd::RDSR2, 0, &[], &mut sr2);
        assert_eq!(sr2, [0x02], "QE bit set");
        f.command(0x42, 0, &[], &mut []);
        assert_eq!(f.unknown_commands(), &[(0x42, 1)]);
    }
}
