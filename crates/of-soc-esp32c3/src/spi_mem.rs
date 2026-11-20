//! SPI0 and SPI1, the SPI memory controllers in front of the flash chip
//! (TRM chapter "SPI Memory Controller"; register offsets as in ESP-IDF's
//! `spi_mem_reg.h`).
//!
//! SPI1 is what software uses to send commands to the flash (the ROM's
//! `esp_rom_spiflash_*` functions, bootloaders, flash drivers); SPI0 serves
//! the cache. Both are modelled the same way: every register reads back the
//! last write, and setting one of the operation bits in `SPI_MEM_CMD_REG`
//! runs the transaction on the [`SpiFlash`] at once and clears the bit
//! (the operation is "done" by the next read). Supported operations: the
//! fixed commands (`FLASH_READ`, `WREN`, `WRDI`, `RDID`, `RDSR`, `WRSR`,
//! `PP`, `SE`, `BE`, `CE`, `DP`, `RES`, `HPM`, `PE`) and user transactions
//! (`USR`: command, address, dummy, MOSI and MISO phases from `USER`,
//! `USER1`, `USER2`, `ADDR`, `MOSI_DLEN`, `MISO_DLEN` and the 16 data words
//! `W0`..`W15`). Line modes (DIO, QIO) and clocks change nothing: data is
//! moved instantly and transfer time is not modelled.

use crate::flash::{SpiFlash, cmd};

/// Register offsets.
pub mod reg {
    /// Operation bits.
    pub const CMD: u32 = 0x000;
    /// Address.
    pub const ADDR: u32 = 0x004;
    /// Read mode and misc control.
    pub const CTRL: u32 = 0x008;
    /// User transaction phases.
    pub const USER: u32 = 0x018;
    /// Address bit length and dummy cycles.
    pub const USER1: u32 = 0x01c;
    /// Command bit length and value.
    pub const USER2: u32 = 0x020;
    /// MOSI data bit length minus one.
    pub const MOSI_DLEN: u32 = 0x024;
    /// MISO data bit length minus one.
    pub const MISO_DLEN: u32 = 0x028;
    /// Status value for `RDSR`/`WRSR`.
    pub const RD_STATUS: u32 = 0x02c;
    /// First data word.
    pub const W0: u32 = 0x058;
    /// Last data word.
    pub const W15: u32 = 0x094;
    /// Version register.
    pub const DATE: u32 = 0x3fc;
}

/// Bits of `SPI_MEM_CMD_REG`.
pub mod op {
    /// Read flash (address in `ADDR`, length in `MISO_DLEN`).
    pub const FLASH_READ: u32 = 1 << 31;
    /// Write enable.
    pub const WREN: u32 = 1 << 30;
    /// Write disable.
    pub const WRDI: u32 = 1 << 29;
    /// Read JEDEC ID into `W0`.
    pub const RDID: u32 = 1 << 28;
    /// Read status into `RD_STATUS`.
    pub const RDSR: u32 = 1 << 27;
    /// Write status from `RD_STATUS`.
    pub const WRSR: u32 = 1 << 26;
    /// Page program from `W0`.. (length in `ADDR[31:24]` or `MOSI_DLEN`).
    pub const PP: u32 = 1 << 25;
    /// Sector erase.
    pub const SE: u32 = 1 << 24;
    /// Block erase.
    pub const BE: u32 = 1 << 23;
    /// Chip erase.
    pub const CE: u32 = 1 << 22;
    /// Deep power-down.
    pub const DP: u32 = 1 << 21;
    /// Release from deep power-down.
    pub const RES: u32 = 1 << 20;
    /// High performance mode.
    pub const HPM: u32 = 1 << 19;
    /// User-defined transaction.
    pub const USR: u32 = 1 << 18;
    /// Page erase.
    pub const PE: u32 = 1 << 17;
    /// All operation bits (they clear when the operation is done).
    pub const ALL: u32 = 0xfffe_0000;
}

/// `USER` register phase bits.
mod user {
    pub const COMMAND: u32 = 1 << 31;
    pub const ADDR: u32 = 1 << 30;
    pub const MISO: u32 = 1 << 28;
    pub const MOSI: u32 = 1 << 27;
}

const REGS: usize = 0x400 / 4;

/// One SPI memory controller (SPI0 or SPI1).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpiMem {
    regs: Vec<u32>,
    /// Operations run so far (for tests and diagnostics).
    pub ops: u64,
}

impl Default for SpiMem {
    fn default() -> Self {
        Self::new()
    }
}

impl SpiMem {
    /// Reset state: everything 0 except the version register.
    pub fn new() -> Self {
        let mut regs = vec![0; REGS];
        regs[(reg::DATE / 4) as usize] = 0x0200_7130;
        Self { regs, ops: 0 }
    }

    fn r(&self, off: u32) -> u32 {
        self.regs[(off / 4) as usize]
    }

    /// Read a register (offset within the block).
    pub fn read(&self, off: u32) -> Option<u32> {
        self.regs.get((off / 4) as usize).copied()
    }

    /// Write a register; operation bits in `CMD` run against `flash`.
    pub fn write(&mut self, off: u32, v: u32, flash: &mut SpiFlash) -> Option<()> {
        let slot = self.regs.get_mut((off / 4) as usize)?;
        match off {
            reg::DATE => {}
            reg::CMD => {
                *slot = v & !op::ALL;
                if v & op::ALL != 0 {
                    self.run(v & op::ALL, flash);
                }
            }
            _ => *slot = v,
        }
        Some(())
    }

    fn data_bytes(&self, n: usize) -> Vec<u8> {
        (0..n.min(64))
            .map(|i| (self.r(reg::W0 + (i as u32 & !3)) >> (8 * (i & 3))) as u8)
            .collect()
    }

    fn set_data_bytes(&mut self, bytes: &[u8]) {
        for (i, b) in bytes.iter().take(64).enumerate() {
            let w = &mut self.regs[((reg::W0 / 4) as usize) + i / 4];
            let shift = 8 * (i & 3);
            *w = (*w & !(0xff << shift)) | (u32::from(*b) << shift);
        }
    }

    fn bits_to_bytes(dlen: u32) -> usize {
        (((dlen & 0x3ff) + 1).div_ceil(8)) as usize
    }

    fn run(&mut self, ops: u32, flash: &mut SpiFlash) {
        self.ops += 1;
        let addr24 = self.r(reg::ADDR) & 0x00ff_ffff;
        let status = self.r(reg::RD_STATUS) & 0xffff;
        let mut miso = [0u8; 64];
        if ops & op::USR != 0 {
            self.run_user(flash);
        }
        if ops & op::FLASH_READ != 0 {
            let n = Self::bits_to_bytes(self.r(reg::MISO_DLEN));
            flash.command(cmd::READ, addr24, &[], &mut miso[..n]);
            self.set_data_bytes(&miso[..n]);
        }
        if ops & op::WREN != 0 {
            flash.command(cmd::WREN, 0, &[], &mut []);
        }
        if ops & op::WRDI != 0 {
            flash.command(cmd::WRDI, 0, &[], &mut []);
        }
        if ops & op::RDID != 0 {
            flash.command(cmd::RDID, 0, &[], &mut miso[..3]);
            self.regs[(reg::W0 / 4) as usize] = u32::from_le_bytes([miso[0], miso[1], miso[2], 0]);
        }
        if ops & op::RDSR != 0 {
            let s = u32::from(flash.status());
            let slot = &mut self.regs[(reg::RD_STATUS / 4) as usize];
            *slot = (*slot & !0xffff) | s;
        }
        if ops & op::WRSR != 0 {
            flash.command(cmd::WRSR, 0, &(status as u16).to_le_bytes(), &mut []);
        }
        if ops & op::PP != 0 {
            let len = match self.r(reg::ADDR) >> 24 {
                0 => Self::bits_to_bytes(self.r(reg::MOSI_DLEN)),
                n => n as usize,
            };
            let data = self.data_bytes(len);
            flash.command(cmd::PP, addr24, &data, &mut []);
        }
        if ops & op::SE != 0 {
            flash.command(cmd::SE, addr24, &[], &mut []);
        }
        if ops & op::BE != 0 {
            flash.command(cmd::BE, addr24, &[], &mut []);
        }
        if ops & op::CE != 0 {
            flash.command(cmd::CE, 0, &[], &mut []);
        }
        if ops & op::PE != 0 {
            flash.command(cmd::PE, addr24, &[], &mut []);
        }
    }

    fn run_user(&mut self, flash: &mut SpiFlash) {
        let user = self.r(reg::USER);
        let opcode = if user & user::COMMAND != 0 {
            self.r(reg::USER2) as u8
        } else {
            return;
        };
        let addr = if user & user::ADDR != 0 {
            let bits = (self.r(reg::USER1) >> 26) + 1;
            let a = self.r(reg::ADDR);
            if bits >= 32 { a } else { a & ((1 << bits) - 1) }
        } else {
            0
        };
        let mosi = if user & user::MOSI != 0 {
            self.data_bytes(Self::bits_to_bytes(self.r(reg::MOSI_DLEN)))
        } else {
            Vec::new()
        };
        let n = if user & user::MISO != 0 {
            Self::bits_to_bytes(self.r(reg::MISO_DLEN)).min(64)
        } else {
            0
        };
        let mut miso = [0u8; 64];
        flash.command(opcode, addr, &mosi, &mut miso[..n]);
        if n > 0 {
            self.set_data_bytes(&miso[..n]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(s: &mut SpiMem, f: &mut SpiFlash, off: u32, v: u32) {
        s.write(off, v, f).unwrap();
    }

    #[test]
    fn fixed_commands_read_id_status_and_data() {
        let (mut s, mut f) = (SpiMem::new(), SpiFlash::new(4 << 20));
        f.write_raw(0x1000, &[1, 2, 3, 4, 5, 6, 7, 8]);
        w(&mut s, &mut f, reg::CMD, op::RDID);
        assert_eq!(s.read(reg::CMD), Some(0), "operation bit clears when done");
        assert_eq!(s.read(reg::W0), Some(0x0016_40c8));
        w(&mut s, &mut f, reg::ADDR, 0x1000);
        w(&mut s, &mut f, reg::MISO_DLEN, 8 * 6 - 1);
        w(&mut s, &mut f, reg::CMD, op::FLASH_READ);
        assert_eq!(s.read(reg::W0), Some(0x0403_0201));
        assert_eq!(s.read(reg::W0 + 4).map(|v| v & 0xffff), Some(0x0605));
        w(&mut s, &mut f, reg::CMD, op::WREN);
        w(&mut s, &mut f, reg::CMD, op::RDSR);
        assert_eq!(s.read(reg::RD_STATUS), Some(2), "WEL");
    }

    #[test]
    fn user_transactions_program_erase_and_read() {
        let (mut s, mut f) = (SpiMem::new(), SpiFlash::new(1 << 20));
        let user = |s: &mut SpiMem, f: &mut SpiFlash, opcode: u32, phases: u32| {
            w(s, f, reg::USER, user::COMMAND | phases);
            w(s, f, reg::USER2, (7 << 28) | opcode);
            w(s, f, reg::CMD, op::USR);
        };
        w(&mut s, &mut f, reg::USER1, 23 << 26);
        user(&mut s, &mut f, 0x06, 0);
        w(&mut s, &mut f, reg::ADDR, 0x0002_0010);
        w(&mut s, &mut f, reg::MOSI_DLEN, 31);
        w(&mut s, &mut f, reg::W0, 0x4433_2211);
        user(&mut s, &mut f, 0x02, user::ADDR | user::MOSI);
        assert_eq!(f.bytes()[0x2_0010..0x2_0014], [0x11, 0x22, 0x33, 0x44]);
        w(&mut s, &mut f, reg::W0, 0);
        w(&mut s, &mut f, reg::MISO_DLEN, 15);
        user(&mut s, &mut f, 0x03, user::ADDR | user::MISO);
        assert_eq!(s.read(reg::W0), Some(0x2211));
        user(&mut s, &mut f, 0x06, 0);
        user(&mut s, &mut f, 0x20, user::ADDR);
        assert_eq!(f.byte(0x2_0010), 0xff, "sector erased");
        assert_eq!(s.ops, 5);
    }
}
