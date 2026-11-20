//! Firmware loading: ELF images, ESP flash images and partition tables now; UF2 and DWARF
//! symbols later.
//!
//! [`elf::ElfImage::parse`] reads a 32-bit little-endian RISC-V ELF
//! executable as produced by ESP-IDF, Arduino-ESP32, esp-hal or a bare
//! assembler: the entry point, the loadable segments and the symbol table.
//! [`esp_image::EspImage::parse`] reads an ESP application or bootloader
//! image (`esptool elf2image` output) and checks its checksum and SHA-256.
//! [`partition::PartitionTable::parse`] reads the partition table the
//! bootloader finds at flash offset `0x8000`.

pub mod elf;
pub mod esp_image;
pub mod md5;
pub mod partition;
pub mod sha256;

pub use elf::{ElfError, ElfImage, Section, Segment, Symbol};
pub use esp_image::{EspImage, ImageError, ImageSegment};
pub use partition::{Partition, PartitionError, PartitionTable};
pub use sha256::sha256;
