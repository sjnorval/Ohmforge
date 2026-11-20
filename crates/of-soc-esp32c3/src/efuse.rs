//! eFuse controller (`0x6000_8800`; register offsets as in ESP-IDF's
//! `efuse_reg.h`).
//!
//! Models a blank chip of wafer revision v0.3 (the ECO3 die whose ROM
//! Ohmforge loads): every read-out register is 0 except the wafer version,
//! so flash encryption, secure boot and download-mode restrictions are all
//! off. `EFUSE_CMD_REG` read and program commands finish at once (the bit
//! clears); programming does not burn anything. Other registers read back
//! the last write.

/// Register offsets from [`crate::map::EFUSE_BASE`].
pub mod reg {
    /// MAC, SPI pad configuration and wafer version (minor, low bits 20:18).
    pub const RD_MAC_SPI_SYS_3: u32 = 0x050;
    /// Controller state (bits 3:0; 1 = idle).
    pub const STATUS: u32 = 0x1d0;
    /// Read (bit 0) and program (bit 1) commands.
    pub const CMD: u32 = 0x1d4;
    /// Version register.
    pub const DATE: u32 = 0x1fc;
}

const REGS: usize = 0x200 / 4;
/// Wafer version minor 3 (`EFUSE_WAFER_VERSION_MINOR_LO`).
const WAFER_V0_3: u32 = 3 << 18;

/// The eFuse controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Efuse {
    regs: Vec<u32>,
}

impl Default for Efuse {
    fn default() -> Self {
        Self::new()
    }
}

impl Efuse {
    /// A blank v0.3 chip.
    pub fn new() -> Self {
        let mut regs = vec![0; REGS];
        regs[(reg::RD_MAC_SPI_SYS_3 / 4) as usize] = WAFER_V0_3;
        regs[(reg::STATUS / 4) as usize] = 1;
        regs[(reg::DATE / 4) as usize] = 0x0200_7200;
        Self { regs }
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        self.regs.get((off / 4) as usize).copied()
    }

    /// Write a register. Read-out and status registers ignore writes.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        let slot = self.regs.get_mut((off / 4) as usize)?;
        match off {
            // Program buffer (0x00..0x2c) and configuration (0x1c8..) are writable.
            0x000..0x02c | 0x1c8 | 0x1cc | 0x1d8..=0x1f8 => *slot = v,
            reg::CMD => *slot = v & !0b11,
            _ => {}
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn blank_chip_with_wafer_version_and_instant_commands() {
        let mut e = Efuse::new();
        assert_eq!(e.read(reg::RD_MAC_SPI_SYS_3).map(|v| (v >> 18) & 7), Some(3));
        assert_eq!(e.read(0x30), Some(0), "no secure boot, no encryption");
        e.write(reg::CMD, 1).unwrap();
        assert_eq!(e.read(reg::CMD), Some(0), "read command done");
        e.write(reg::RD_MAC_SPI_SYS_3, 0).unwrap();
        assert_eq!(e.read(reg::RD_MAC_SPI_SYS_3), Some(3 << 18), "read-out is read-only");
        assert_eq!(e.read(reg::STATUS), Some(1));
    }
}
