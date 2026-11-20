//! SHA accelerator (`0x6003_B000`), typical (CPU-fed) SHA mode only.
//!
//! The ROM hashes app and bootloader images with it. Software writes one
//! 64-byte block to `M_MEM`, then `START` (begin a new digest) or
//! `CONTINUE` (carry on from `H_MEM`); the result is in `H_MEM`. Both
//! memories hold bytes in message order, so a word reads as the
//! little-endian view of big-endian hash words. SHA-256 and SHA-224 are
//! modelled; SHA-1 (`MODE` 0) and DMA mode are not (logged as
//! unsupported, `H_MEM` left unchanged). Operations finish at once
//! (`BUSY` always reads 0; real hardware takes about 80 cycles per block).

use of_firmware::sha256::{H0, H0_224, compress};

/// Register offsets.
pub mod reg {
    /// Algorithm: 0 SHA-1, 1 SHA-224, 2 SHA-256.
    pub const MODE: u32 = 0x00;
    /// Start a new digest with the block in `M_MEM`.
    pub const START: u32 = 0x10;
    /// Continue the digest in `H_MEM` with the block in `M_MEM`.
    pub const CONTINUE: u32 = 0x14;
    /// 1 while an operation runs.
    pub const BUSY: u32 = 0x18;
    /// DMA-mode start.
    pub const DMA_START: u32 = 0x1c;
    /// DMA-mode continue.
    pub const DMA_CONTINUE: u32 = 0x20;
    /// Version.
    pub const DATE: u32 = 0x2c;
    /// Hash state, 8 words.
    pub const H_MEM: u32 = 0x40;
    /// Message block, 16 words.
    pub const M_MEM: u32 = 0x80;
}

const DATE: u32 = 0x2020_0616;
const REGS: usize = 0x100 / 4;

/// The accelerator.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Sha {
    regs: Vec<u32>,
    /// Blocks hashed so far.
    pub blocks: u64,
    /// Operations asked for in unsupported modes (SHA-1, DMA).
    pub unsupported: u64,
}

impl Default for Sha {
    fn default() -> Self {
        Self::new()
    }
}

impl Sha {
    /// Reset state.
    pub fn new() -> Self {
        Self {
            regs: vec![0; REGS],
            blocks: 0,
            unsupported: 0,
        }
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            reg::BUSY | reg::START | reg::CONTINUE | reg::DMA_START | reg::DMA_CONTINUE => Some(0),
            reg::DATE => Some(DATE),
            _ => self.regs.get((off / 4) as usize).copied(),
        }
    }

    /// Write a register (`START`/`CONTINUE` hash the block at once).
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        match off {
            reg::START | reg::CONTINUE => self.run(off == reg::START),
            reg::DMA_START | reg::DMA_CONTINUE => self.unsupported += 1,
            reg::BUSY | reg::DATE => {}
            _ => *self.regs.get_mut((off / 4) as usize)? = v,
        }
        Some(())
    }

    fn run(&mut self, start: bool) {
        let mut h = match self.regs[(reg::MODE / 4) as usize] & 7 {
            1 if start => H0_224,
            2 if start => H0,
            1 | 2 => std::array::from_fn(|i| self.regs[(reg::H_MEM / 4) as usize + i].swap_bytes()),
            _ => {
                self.unsupported += 1;
                return;
            }
        };
        let mut block = [0u8; 64];
        for (i, chunk) in block.as_chunks_mut::<4>().0.iter_mut().enumerate() {
            *chunk = self.regs[(reg::M_MEM / 4) as usize + i].to_le_bytes();
        }
        compress(&mut h, &block);
        for (i, w) in h.iter().enumerate() {
            self.regs[(reg::H_MEM / 4) as usize + i] = w.swap_bytes();
        }
        self.blocks += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_block_sha256_of_abc() {
        let mut sha = Sha::new();
        let mut block = [0u8; 64];
        block[..3].copy_from_slice(b"abc");
        block[3] = 0x80;
        block[63] = 24;
        for (i, w) in block.as_chunks::<4>().0.iter().enumerate() {
            sha.write(reg::M_MEM + 4 * i as u32, u32::from_le_bytes(*w));
        }
        sha.write(reg::MODE, 2);
        sha.write(reg::START, 1);
        assert_eq!(sha.read(reg::BUSY), Some(0));
        let digest: Vec<u8> = (0..8)
            .flat_map(|i| sha.read(reg::H_MEM + 4 * i).unwrap_or(0).to_le_bytes())
            .collect();
        assert_eq!(digest, of_firmware::sha256(b"abc"));
    }
}
