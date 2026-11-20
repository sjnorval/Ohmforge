//! ESP application and bootloader flash images (the `.bin` files written
//! by `esptool elf2image` and flashed by `esptool write_flash`).
//!
//! Layout (ESP-IDF `esp_app_format.h`): a 24-byte header (magic `0xE9`,
//! segment count, SPI mode, SPI speed and size, entry point, then the
//! extended header with write-protect pin, SPI pin drive, chip id, minimum
//! and maximum chip revision and the `hash_appended` flag), then each
//! segment as load address, length and data. Zero padding follows up to
//! the last byte of a 16-byte block, which holds the checksum (`0xEF` XOR
//! every segment data byte). When `hash_appended` is 1, a SHA-256 of all of
//! the above follows. Everything is little-endian.
//!
//! [`EspImage::parse`] checks the magic, every length, the checksum and the
//! digest; [`EspImage::to_bytes`] writes the same format and
//! [`EspImage::from_elf`] builds an image from an ELF's segments.
//!
//! Segments that execute in place from flash (ESP32-C3 IROM `0x4200_0000`
//! or DROM `0x3C00_0000`) are mapped by the cache MMU in 64 KiB pages, so
//! their data must sit in flash at an offset congruent to their address
//! modulo 64 KiB. [`EspImage::align_flash_segments`] inserts padding
//! segments (load address 0, which loaders skip) to make that so, as
//! `esptool elf2image` does.

use thiserror::Error;

use crate::elf::ElfImage;
use crate::sha256::sha256;

/// First byte of every image.
pub const MAGIC: u8 = 0xe9;
/// Checksum seed.
pub const CHECKSUM_SEED: u8 = 0xef;
const HEADER_LEN: usize = 24;
const MAX_SEGMENTS: u8 = 16;
/// Cache MMU page size: flash-mapped segments must keep
/// `flash offset ≡ address (mod MMU_PAGE)`.
pub const MMU_PAGE: u32 = 0x1_0000;

/// Why an image was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ImageError {
    /// The first byte is not `0xE9`.
    #[error("not an ESP image (magic {0:#04x})")]
    BadMagic(u8),
    /// The file ends inside the header, a segment or the trailer.
    #[error("truncated ESP image: {0}")]
    Truncated(String),
    /// More segments than the ROM accepts (16).
    #[error("too many segments: {0}")]
    TooManySegments(u8),
    /// The stored checksum does not match the data.
    #[error("checksum mismatch: stored {stored:#04x}, computed {computed:#04x}")]
    Checksum {
        /// Byte in the image.
        stored: u8,
        /// XOR of the segment data.
        computed: u8,
    },
    /// The appended SHA-256 does not match.
    #[error("SHA-256 digest mismatch")]
    Digest,
}

/// One image segment.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageSegment {
    /// Load address.
    pub addr: u32,
    /// Bytes.
    pub data: Vec<u8>,
}

/// A parsed ESP image.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EspImage {
    /// SPI flash mode (0 QIO, 1 QOUT, 2 DIO, 3 DOUT).
    pub spi_mode: u8,
    /// SPI flash speed code (low nibble of byte 3).
    pub spi_speed: u8,
    /// SPI flash size code (high nibble of byte 3).
    pub spi_size: u8,
    /// Entry point.
    pub entry: u32,
    /// Write-protect pin (`0xEE` = not used).
    pub wp_pin: u8,
    /// SPI pin drive settings.
    pub spi_pin_drv: [u8; 3],
    /// Chip id: 0 ESP32, 2 ESP32-S2, 5 ESP32-C3, 9 ESP32-S3, 13 ESP32-C6.
    pub chip_id: u16,
    /// Minimum chip revision (legacy byte).
    pub min_chip_rev: u8,
    /// Minimum chip revision, `major * 100 + minor`.
    pub min_chip_rev_full: u16,
    /// Maximum chip revision, `major * 100 + minor`.
    pub max_chip_rev_full: u16,
    /// Whether a SHA-256 digest follows the checksum.
    pub hash_appended: bool,
    /// Segments in file order.
    pub segments: Vec<ImageSegment>,
}

fn take<'a>(bytes: &'a [u8], at: usize, len: usize, what: &str) -> Result<&'a [u8], ImageError> {
    bytes
        .get(at..at.checked_add(len).ok_or_else(|| ImageError::Truncated(what.into()))?)
        .ok_or_else(|| ImageError::Truncated(what.into()))
}

fn u16_at(b: &[u8], at: usize) -> u16 {
    u16::from_le_bytes([b[at], b[at + 1]])
}

fn u32_at(b: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([b[at], b[at + 1], b[at + 2], b[at + 3]])
}

impl EspImage {
    /// An empty image for `chip_id` with esptool's usual defaults (DIO,
    /// 40 MHz, size code 1 = 2 MB, any chip revision) and a digest.
    pub fn new(chip_id: u16, entry: u32) -> Self {
        Self {
            spi_mode: 2,
            spi_speed: 0,
            spi_size: 1,
            entry,
            wp_pin: 0xee,
            spi_pin_drv: [0; 3],
            chip_id,
            min_chip_rev: 0,
            min_chip_rev_full: 0,
            max_chip_rev_full: 0xffff,
            hash_appended: true,
            segments: Vec::new(),
        }
    }

    /// An image holding the loadable bytes of `elf` (one segment per
    /// `PT_LOAD` with file data, in ELF order; `.bss` is left to the
    /// startup code, as esptool does).
    pub fn from_elf(elf: &ElfImage, chip_id: u16) -> Self {
        let mut img = Self::new(chip_id, elf.entry);
        img.segments = elf
            .segments
            .iter()
            .filter(|s| !s.data.is_empty())
            .map(|s| ImageSegment {
                addr: s.vaddr,
                data: s.data.clone(),
            })
            .collect();
        img
    }

    /// Offset of each segment's data from the start of the image.
    pub fn data_offsets(&self) -> Vec<u32> {
        let mut pos = HEADER_LEN as u32;
        self.segments
            .iter()
            .map(|s| {
                let at = pos + 8;
                pos = at + s.data.len() as u32;
                at
            })
            .collect()
    }

    /// Insert padding segments so that every segment for which
    /// `flash_mapped(addr)` holds has its data at a flash offset congruent
    /// to its address modulo [`MMU_PAGE`], given that the image starts at
    /// `image_offset` in flash. Padding segments load at address 0 and
    /// hold zeros. Fails if that needs more than 16 segments.
    pub fn align_flash_segments(
        &mut self,
        image_offset: u32,
        flash_mapped: impl Fn(u32) -> bool,
    ) -> Result<(), ImageError> {
        let mut out: Vec<ImageSegment> = Vec::with_capacity(self.segments.len());
        let mut pos = image_offset.wrapping_add(HEADER_LEN as u32);
        for seg in std::mem::take(&mut self.segments) {
            if flash_mapped(seg.addr) && (pos + 8) % MMU_PAGE != seg.addr % MMU_PAGE {
                let pad = seg.addr.wrapping_sub(pos + 16) % MMU_PAGE;
                out.push(ImageSegment {
                    addr: 0,
                    data: vec![0; pad as usize],
                });
                pos += 8 + pad;
            }
            pos += 8 + seg.data.len() as u32;
            out.push(seg);
        }
        if out.len() > usize::from(MAX_SEGMENTS) {
            return Err(ImageError::TooManySegments(out.len() as u8));
        }
        self.segments = out;
        Ok(())
    }

    /// The checksum over the segments' data.
    pub fn checksum(&self) -> u8 {
        self.segments
            .iter()
            .flat_map(|s| s.data.iter())
            .fold(CHECKSUM_SEED, |acc, b| acc ^ b)
    }

    /// Parse an image from the start of `bytes` (trailing bytes, such as
    /// the rest of a flash partition, are ignored).
    pub fn parse(bytes: &[u8]) -> Result<Self, ImageError> {
        let h = take(bytes, 0, HEADER_LEN, "header")?;
        if h[0] != MAGIC {
            return Err(ImageError::BadMagic(h[0]));
        }
        let count = h[1];
        if count > MAX_SEGMENTS {
            return Err(ImageError::TooManySegments(count));
        }
        let mut img = Self {
            spi_mode: h[2],
            spi_speed: h[3] & 0xf,
            spi_size: h[3] >> 4,
            entry: u32_at(h, 4),
            wp_pin: h[8],
            spi_pin_drv: [h[9], h[10], h[11]],
            chip_id: u16_at(h, 12),
            min_chip_rev: h[14],
            min_chip_rev_full: u16_at(h, 15),
            max_chip_rev_full: u16_at(h, 17),
            hash_appended: h[23] == 1,
            segments: Vec::with_capacity(usize::from(count)),
        };
        let mut pos = HEADER_LEN;
        for n in 0..count {
            let sh = take(bytes, pos, 8, &format!("segment {n} header"))?;
            let (addr, len) = (u32_at(sh, 0), u32_at(sh, 4) as usize);
            let data = take(bytes, pos + 8, len, &format!("segment {n} data ({len} bytes)"))?;
            img.segments.push(ImageSegment {
                addr,
                data: data.to_vec(),
            });
            pos += 8 + len;
        }
        // Checksum in the last byte of the 16-byte block.
        let sum_at = pos | 15;
        let stored = take(bytes, sum_at, 1, "checksum")?[0];
        let computed = img.checksum();
        if stored != computed {
            return Err(ImageError::Checksum { stored, computed });
        }
        if img.hash_appended {
            let digest = take(bytes, sum_at + 1, 32, "SHA-256 digest")?;
            if sha256(&bytes[..=sum_at]) != digest {
                return Err(ImageError::Digest);
            }
        }
        Ok(img)
    }

    /// Serialise (header, segments, padding, checksum, optional digest).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = vec![
            MAGIC,
            self.segments.len() as u8,
            self.spi_mode,
            (self.spi_size << 4) | (self.spi_speed & 0xf),
        ];
        out.extend_from_slice(&self.entry.to_le_bytes());
        out.push(self.wp_pin);
        out.extend_from_slice(&self.spi_pin_drv);
        out.extend_from_slice(&self.chip_id.to_le_bytes());
        out.push(self.min_chip_rev);
        out.extend_from_slice(&self.min_chip_rev_full.to_le_bytes());
        out.extend_from_slice(&self.max_chip_rev_full.to_le_bytes());
        out.extend_from_slice(&[0; 4]);
        out.push(u8::from(self.hash_appended));
        for s in &self.segments {
            out.extend_from_slice(&s.addr.to_le_bytes());
            out.extend_from_slice(&(s.data.len() as u32).to_le_bytes());
            out.extend_from_slice(&s.data);
        }
        out.resize(out.len() | 15, 0);
        out.push(self.checksum());
        if self.hash_appended {
            let digest = sha256(&out);
            out.extend_from_slice(&digest);
        }
        out
    }
}
