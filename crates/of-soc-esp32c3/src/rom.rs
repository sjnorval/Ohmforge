//! The ESP32-C3 mask ROM, built from Espressif's ROM ELF files.
//!
//! Espressif publishes the ROM of each chip revision as an ELF in
//! [esp-rom-elfs](https://github.com/espressif/esp-rom-elfs) (Apache-2.0;
//! the ROM code itself is not open source). Ohmforge never commits or
//! bundles these files: `scripts/fetch-esp-rom-elfs.sh` (or `.ps1`) downloads
//! the pinned release, checks its SHA-256 and unpacks it outside `target/`,
//! and the ROM tests skip when it is absent.
//!
//! The ELF is not a plain memory image. Its code (`.fixed.text`,
//! `.init.text`, `.bt_text`, `.text`) and read-only data (at `0x3ff1_9c00`,
//! the data bus view of ROM1) are mostly allocated sections, but:
//!
//! - A few ROM windows (notably `.rodata.interface`, which ends with
//!   `ets_rom_layout_p` → `ets_rom_layout`) are `PROGBITS` in the ROM
//!   address range **without** `SHF_ALLOC`. They still sit in the physical
//!   mask ROM; ESP-IDF heap init reads `ets_rom_layout_p`, so those bytes
//!   must be present or a full IDF app panics with a load fault at address 4.
//! - The initial values of the ROM's RAM variables sit in non-allocated
//!   `.data*` sections at their RAM addresses. In the real ROM those bytes
//!   are stored after the code (`_rom_store`) and copied to RAM at reset by
//!   the startup code, which walks the table at `_rom_store_table` (entries
//!   of RAM start, RAM end, ROM source, padding; up to `_data_end`).
//!
//! [`RomImage::from_elf`] places every ROM-window `PROGBITS` (allocated or
//! not), then rebuilds the `.data*` store layout so the ROM's own startup
//! copies the right values.

use of_firmware::ElfImage;
use of_firmware::elf::{SHF_ALLOC, SHT_PROGBITS};
use thiserror::Error;

use crate::map::{self, within};

/// The esp-rom-elfs release the tests and scripts are pinned to.
pub const ROM_ELFS_RELEASE: &str = "20260528";
/// SHA-256 of `esp-rom-elfs-<release>.tar.gz`.
pub const ROM_ELFS_TARBALL_SHA256: &str = "caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46";
/// File name of the ROM for ESP32-C3 revision 3 (ECO3, also used by
/// revision 4 chips), the revision Ohmforge models.
pub const C3_ROM_FILE: &str = "esp32c3_rev3_rom.elf";
/// SHA-256 of [`C3_ROM_FILE`] in the pinned release.
pub const C3_ROM_SHA256: &str = "19ac22e08707df926fb0cf4c54795d4067b983fae8635f396ded173a6d78fc3c";

/// Why a ROM ELF could not be turned into a ROM image.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RomError {
    /// The ELF's entry point is not the C3 reset vector.
    #[error("ROM entry point {0:#010x} is not the ESP32-C3 reset vector 0x40000000")]
    Entry(u32),
    /// A symbol the loader needs is missing.
    #[error("ROM ELF has no `{0}` symbol")]
    MissingSymbol(&'static str),
    /// A section or table entry points outside the ROM.
    #[error("ROM ELF {0} is outside the ESP32-C3 ROM")]
    OutOfRange(String),
}

/// The 384 KiB ROM as the instruction bus sees it (`0x4000_0000`).
#[derive(Clone, PartialEq, Eq)]
pub struct RomImage {
    bytes: Vec<u8>,
    /// ROM symbols (function names), for diagnostics and tests.
    pub symbols: Vec<(String, u32)>,
}

impl std::fmt::Debug for RomImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("RomImage")
            .field("bytes", &self.bytes.len())
            .field("symbols", &self.symbols.len())
            .finish()
    }
}

/// Instruction-bus address of a ROM address on either bus.
fn ibus(addr: u32, len: u32) -> Option<u32> {
    if within(map::IROM_INTERNAL, addr, len) {
        Some(addr)
    } else if within(map::DROM_INTERNAL, addr, len) {
        // ROM1 through the data bus.
        Some(addr - map::DROM_INTERNAL.0 + map::IROM_INTERNAL.0 + 0x4_0000)
    } else {
        None
    }
}

fn put(bytes: &mut [u8], addr: u32, data: &[u8], what: &str) -> Result<(), RomError> {
    let a = ibus(addr, data.len() as u32).ok_or_else(|| RomError::OutOfRange(what.to_string()))?;
    let off = (a - map::IROM_INTERNAL.0) as usize;
    bytes[off..off + data.len()].copy_from_slice(data);
    Ok(())
}

impl RomImage {
    /// Build the ROM image from an esp-rom-elfs ELF (see the module docs).
    pub fn from_elf(elf: &ElfImage) -> Result<Self, RomError> {
        if elf.entry != map::IROM_INTERNAL.0 {
            return Err(RomError::Entry(elf.entry));
        }
        let mut bytes = vec![0u8; map::IROM_INTERNAL.1 as usize];
        // Include non-ALLOC ROM `PROGBITS` (e.g. `.rodata.interface`). RAM
        // `.data*` live outside the ROM windows, so `ibus` rejects them here.
        for s in &elf.sections {
            if s.kind == SHT_PROGBITS && !s.data.is_empty() && ibus(s.addr, s.size.max(1)).is_some() {
                put(&mut bytes, s.addr, &s.data, &format!("section {}", s.name))?;
            }
        }
        let sym = |name: &'static str| elf.symbol(name).map(|s| s.value).ok_or(RomError::MissingSymbol(name));
        let (table, end) = (sym("_rom_store_table")?, sym("_data_end")?);
        let word = |bytes: &[u8], addr: u32| -> Result<u32, RomError> {
            let off = ibus(addr, 4)
                .map(|a| (a - map::IROM_INTERNAL.0) as usize)
                .ok_or_else(|| RomError::OutOfRange(format!("data table at {addr:#x}")))?;
            Ok(u32::from_le_bytes([
                bytes[off],
                bytes[off + 1],
                bytes[off + 2],
                bytes[off + 3],
            ]))
        };
        let mut entries = Vec::new();
        let mut at = table;
        while at + 12 <= end {
            entries.push((word(&bytes, at)?, word(&bytes, at + 4)?, word(&bytes, at + 8)?));
            at += 16;
        }
        for s in &elf.sections {
            if s.kind != SHT_PROGBITS || s.flags & SHF_ALLOC != 0 || s.data.is_empty() || !s.name.starts_with(".data") {
                continue;
            }
            let entry = entries
                .iter()
                .find(|&&(lo, hi, _)| s.addr >= lo && s.addr + s.size <= hi);
            if let Some(&(lo, _, src)) = entry {
                put(
                    &mut bytes,
                    src + (s.addr - lo),
                    &s.data,
                    &format!("initial data of {}", s.name),
                )?;
            }
        }
        let symbols = elf.symbols.iter().map(|s| (s.name.clone(), s.value)).collect();
        Ok(Self { bytes, symbols })
    }

    /// The ROM bytes, starting at `0x4000_0000`.
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// Address of a ROM symbol.
    pub fn symbol(&self, name: &str) -> Option<u32> {
        self.symbols.iter().find(|(n, _)| n == name).map(|&(_, v)| v)
    }

    /// The function containing `addr` (nearest symbol at or below it), for
    /// diagnostics.
    pub fn function_at(&self, addr: u32) -> Option<(&str, u32)> {
        self.symbols
            .iter()
            .filter(|&&(_, v)| v <= addr && addr - v < 0x1_0000)
            .max_by_key(|&&(_, v)| v)
            .map(|(n, v)| (n.as_str(), addr - v))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use of_firmware::{Section, Symbol};

    fn section(name: &str, flags: u32, addr: u32, data: Vec<u8>) -> Section {
        Section {
            name: name.into(),
            kind: SHT_PROGBITS,
            flags,
            addr,
            size: data.len() as u32,
            data,
        }
    }

    fn symbol(name: &str, value: u32) -> Symbol {
        Symbol {
            name: name.into(),
            value,
            size: 0,
        }
    }

    /// A miniature ROM: code at the reset vector, read-only data seen
    /// through the data bus, and one RAM variable whose initial value the
    /// unpack table stores at 0x4000_2000.
    fn tiny_rom() -> ElfImage {
        let mut table = Vec::new();
        for w in [0x3fcd_f000u32, 0x3fcd_f010, 0x4000_2000, 0] {
            table.extend_from_slice(&w.to_le_bytes());
        }
        ElfImage {
            entry: 0x4000_0000,
            flags: 0,
            segments: vec![],
            symbols: vec![
                symbol("_rom_store_table", 0x4000_1000),
                symbol("_data_end", 0x4000_1010),
            ],
            sections: vec![
                section(".text", SHF_ALLOC, 0x4000_0000, vec![0x13, 0, 0, 0]),
                section(".rodata", SHF_ALLOC, 0x3ff1_9c00, b"hi".to_vec()),
                section(".table", SHF_ALLOC, 0x4000_1000, table),
                section(".data", 0, 0x3fcd_f004, vec![1, 2, 3, 4]),
            ],
        }
    }

    #[test]
    fn sections_and_initial_data_land_where_the_rom_keeps_them() {
        let rom = RomImage::from_elf(&tiny_rom()).expect("valid");
        let b = rom.bytes();
        assert_eq!(b.len(), 384 << 10);
        assert_eq!(&b[..4], &[0x13, 0, 0, 0]);
        // DBus 0x3ff1_9c00 is ROM1: IBus 0x4005_9c00.
        assert_eq!(&b[0x5_9c00..0x5_9c02], b"hi");
        // RAM 0x3fcd_f004 is 4 bytes into the range stored at 0x4000_2000.
        assert_eq!(&b[0x2004..0x2008], &[1, 2, 3, 4]);
        assert_eq!(rom.symbol("_data_end"), Some(0x4000_1010));
        assert_eq!(rom.function_at(0x4000_1004), Some(("_rom_store_table", 4)));
    }

    #[test]
    fn non_rom_elfs_are_rejected() {
        let mut elf = tiny_rom();
        elf.entry = 0x4200_0000;
        assert_eq!(RomImage::from_elf(&elf), Err(RomError::Entry(0x4200_0000)));
        let mut elf = tiny_rom();
        elf.symbols.clear();
        assert_eq!(
            RomImage::from_elf(&elf),
            Err(RomError::MissingSymbol("_rom_store_table"))
        );
        let mut elf = tiny_rom();
        elf.sections[2].data[8..12].copy_from_slice(&0x5000_0000u32.to_le_bytes());
        assert!(matches!(RomImage::from_elf(&elf), Err(RomError::OutOfRange(_))));
    }
}
