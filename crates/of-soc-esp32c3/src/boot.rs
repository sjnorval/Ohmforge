//! ROM-lite: a fast flash boot that stands in for the mask ROM and the
//! second-stage bootloader.
//!
//! [`crate::Esp32c3::boot_flash`] does what the ROM and ESP-IDF's
//! bootloader would leave behind, in zero simulated time, without running
//! either:
//!
//! 1. **Direct boot.** If flash words 0 and 1 are both `0xAEDB041D`, map
//!    every flash page one to one into both cache windows, enable the
//!    cache and jump to `0x4200_0008` (exactly what the ROM's
//!    `ets_run_direct_boot` does).
//! 2. **Partition table.** Otherwise, if a valid table is at `0x8000`,
//!    load the app partition the bootloader would pick without OTA data
//!    (`factory`, else the lowest `ota_n`, else `test`).
//! 3. **Image at 0.** Otherwise load the `0xE9` image at offset 0 (where
//!    the ROM expects the bootloader) as if it were the app.
//!
//! Loading an app image: segments for RAM are copied; segments in the
//! IROM/DROM windows are mapped from where they sit in flash (their flash
//! offset must be congruent to their address modulo 64 KiB, as
//! `esptool elf2image` arranges); padding segments (address 0) are skipped.
//! The checksum and SHA-256 are checked. Not modelled: secure boot, flash
//! encryption, OTA selection data, the bootloader's clock, watchdog and
//! RTC set-up, and app descriptor checks.
//!
//! To execute the real ROM instead, use [`crate::Esp32c3::power_on_rom`]
//! with a [`crate::rom::RomImage`].

use of_firmware::partition::{PartitionError, PartitionTable, TABLE_OFFSET, TABLE_SIZE};
use of_firmware::{EspImage, ImageError};
use thiserror::Error;

use crate::bus::SocBus;
use crate::cache::{self, MMU_ENTRIES};
use crate::map::{self, within};
use crate::soc::CHIP_ID;

/// Flash words 0 and 1 that ask the ROM for direct boot.
pub const DIRECT_BOOT_MAGIC: u32 = 0xaedb_041d;
/// Where direct boot jumps (just past the two magic words).
pub const DIRECT_BOOT_ENTRY: u32 = 0x4200_0008;

/// What [`crate::Esp32c3::boot_flash`] found and started.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FlashBoot {
    /// Direct boot (magic words at flash 0).
    Direct,
    /// The app in a partition from the table at `0x8000`.
    Partition {
        /// Partition label.
        label: String,
        /// Flash offset of the partition.
        offset: u32,
        /// Entry point.
        entry: u32,
    },
    /// The image at flash offset 0.
    ImageAtZero {
        /// Entry point.
        entry: u32,
    },
}

impl FlashBoot {
    /// The address the hart starts at.
    pub fn entry(&self) -> u32 {
        match self {
            Self::Direct => DIRECT_BOOT_ENTRY,
            Self::Partition { entry, .. } | Self::ImageAtZero { entry } => *entry,
        }
    }
}

/// Why the flash could not be booted.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum FlashBootError {
    /// Neither the direct-boot magic, a partition table nor an image.
    #[error("nothing bootable in flash (first word {0:#010x})")]
    Nothing(u32),
    /// A partition table without an app partition.
    #[error("partition table has no app partition")]
    NoApp,
    /// The partition table is corrupt.
    #[error("partition table: {0}")]
    Partition(#[from] PartitionError),
    /// The app image is corrupt.
    #[error("app image at {offset:#x}: {error}")]
    Image {
        /// Flash offset of the image.
        offset: u32,
        /// What is wrong.
        error: ImageError,
    },
    /// An image built for another chip.
    #[error("image is for chip id {0}, not the ESP32-C3 (5)")]
    WrongChip(u16),
    /// A flash-window segment whose flash offset is not congruent to its
    /// address modulo 64 KiB.
    #[error("segment at {addr:#010x} is at flash {flash:#x}, not aligned to its 64 KiB page")]
    Misaligned {
        /// Segment address.
        addr: u32,
        /// Flash offset of its data.
        flash: u32,
    },
    /// Two segments need one MMU entry to point at different flash pages
    /// (IROM and DROM share the table).
    #[error("segment at {addr:#010x} needs an MMU entry already used for another page")]
    MmuConflict {
        /// Segment address.
        addr: u32,
    },
    /// A RAM segment outside the memory map.
    #[error("segment at {addr:#010x} ({len} bytes) is outside the ESP32-C3 memory map")]
    Unmapped {
        /// Segment address.
        addr: u32,
        /// Length.
        len: u32,
    },
}

fn word(bus: &SocBus, off: usize) -> u32 {
    let b = bus.flash.bytes();
    b.get(off..off + 4)
        .map_or(u32::MAX, |w| u32::from_le_bytes([w[0], w[1], w[2], w[3]]))
}

/// Set up `bus` as the ROM and bootloader would and return what was
/// booted. Leaves the hart alone; the caller jumps to
/// [`FlashBoot::entry`].
pub fn boot(bus: &mut SocBus) -> Result<FlashBoot, FlashBootError> {
    if word(bus, 0) == DIRECT_BOOT_MAGIC && word(bus, 4) == DIRECT_BOOT_MAGIC {
        let pages = (bus.flash.size() / cache::PAGE as usize).min(MMU_ENTRIES);
        for n in 0..pages {
            bus.cache.map(n, n as u32);
        }
        bus.cache.enable();
        return Ok(FlashBoot::Direct);
    }
    let table = &bus.flash.bytes()[TABLE_OFFSET as usize..TABLE_OFFSET as usize + TABLE_SIZE];
    match PartitionTable::parse(table) {
        Ok(t) => {
            let app = t.boot_app().ok_or(FlashBootError::NoApp)?.clone();
            let entry = load_app(bus, app.offset)?;
            Ok(FlashBoot::Partition {
                label: app.label,
                offset: app.offset,
                entry,
            })
        }
        Err(PartitionError::NoTable(..)) if bus.flash.bytes()[0] == of_firmware::esp_image::MAGIC => {
            Ok(FlashBoot::ImageAtZero {
                entry: load_app(bus, 0)?,
            })
        }
        Err(PartitionError::NoTable(..)) => Err(FlashBootError::Nothing(word(bus, 0))),
        Err(e) => Err(e.into()),
    }
}

/// Load the app image at flash `offset`; returns its entry point.
fn load_app(bus: &mut SocBus, offset: u32) -> Result<u32, FlashBootError> {
    let bytes = bus.flash.bytes().get(offset as usize..).unwrap_or_default();
    let img = EspImage::parse(bytes).map_err(|error| FlashBootError::Image { offset, error })?;
    if img.chip_id != CHIP_ID {
        return Err(FlashBootError::WrongChip(img.chip_id));
    }
    let mut wanted: [Option<u32>; MMU_ENTRIES] = [None; MMU_ENTRIES];
    let mut ram = Vec::new();
    for (seg, data_off) in img.segments.iter().zip(img.data_offsets()) {
        let (addr, len) = (seg.addr, seg.data.len() as u32);
        if addr == 0 {
            continue;
        }
        if within(map::IROM_FLASH, addr, len) || within(map::DROM_FLASH, addr, len) {
            let flash = offset + data_off;
            if flash % cache::PAGE != addr % cache::PAGE {
                return Err(FlashBootError::Misaligned { addr, flash });
            }
            let linear = addr & (map::IROM_FLASH.1 - 1);
            let first_page = (flash - linear % cache::PAGE) / cache::PAGE;
            for (k, n) in (linear / cache::PAGE..(linear + len).div_ceil(cache::PAGE)).enumerate() {
                let page = first_page + k as u32;
                match wanted[n as usize] {
                    Some(p) if p != page => return Err(FlashBootError::MmuConflict { addr }),
                    _ => wanted[n as usize] = Some(page),
                }
            }
        } else {
            ram.push((addr, seg.data.clone()));
        }
    }
    for (addr, data) in ram {
        bus.load(addr, &data).map_err(|_| FlashBootError::Unmapped {
            addr,
            len: data.len() as u32,
        })?;
    }
    for (n, page) in wanted.iter().enumerate() {
        if let Some(p) = page {
            bus.cache.map(n, *p);
        }
    }
    bus.cache.enable();
    Ok(img.entry)
}
