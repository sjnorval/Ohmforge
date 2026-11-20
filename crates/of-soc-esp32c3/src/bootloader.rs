//! The ESP-IDF second-stage bootloader that espflash ships.
//!
//! esp-hal apps follow the ESP-IDF boot flow: the ROM loads a second-stage
//! bootloader from flash offset 0, which reads the partition table at
//! `0x8000`, loads the app image (`0x10000` for espflash's default table),
//! maps its flash segments through the cache MMU and jumps to it.
//! `espflash save-image --merge` and `espflash flash` write a prebuilt
//! bootloader that espflash carries for each chip, built from ESP-IDF
//! (release/v6.1 for espflash 4.7.0; ESP-IDF is Apache-2.0). It is
//! Espressif's code, so Ohmforge never commits it: the tests fetch the
//! pinned file with `scripts/fetch-esp-rom-elfs.sh` (or `.ps1`), check its
//! SHA-256 and build what espflash writes at flash offset 0 with
//! [`espflash_flash_image`].

use of_firmware::{EspImage, ImageError};

/// The espflash release the bootloader is taken from.
pub const ESPFLASH_RELEASE: &str = "v4.7.0";
/// Name of the fetched ESP32-C3 bootloader (espflash's
/// `espflash/resources/bootloaders/esp32c3-bootloader.bin` at
/// [`ESPFLASH_RELEASE`]).
pub const C3_BOOTLOADER_FILE: &str = "espflash-v4.7.0-esp32c3-bootloader.bin";
/// SHA-256 of [`C3_BOOTLOADER_FILE`].
pub const C3_BOOTLOADER_SHA256: &str = "7c04141e1fd8da5a391e8004ffe3d8ee8e20eeb5baf9bd3abc05c03a17b1ed09";

/// espflash's flash header for a 4 MB flash at 40 MHz (image header byte
/// 3: size code 2, speed code 0).
const FLASH_4MB: u8 = 2;

/// The bootloader as espflash writes it at flash offset 0 for a 4 MB flash:
/// the image header's flash size and speed set (4 MB, 40 MHz) and the
/// image's SHA-256 recomputed, everything else unchanged.
pub fn espflash_flash_image(bootloader: &[u8]) -> Result<Vec<u8>, ImageError> {
    let mut img = EspImage::parse(bootloader)?;
    img.spi_size = FLASH_4MB;
    img.spi_speed = 0;
    Ok(img.to_bytes())
}

#[cfg(test)]
mod tests {
    use of_firmware::{ImageSegment, sha256};

    use super::*;

    #[test]
    fn flash_header_and_digest_are_rewritten() {
        let mut img = EspImage::new(5, 0x403c_e000);
        img.spi_size = 1;
        img.spi_speed = 0xf;
        img.segments = vec![ImageSegment {
            addr: 0x403c_e000,
            data: vec![0x13; 64],
        }];
        let raw = img.to_bytes();
        let out = espflash_flash_image(&raw).unwrap();
        assert_eq!(out.len(), raw.len());
        assert_eq!((raw[3], out[3]), (0x1f, 0x20));
        let n = out.len() - 32;
        assert_eq!(out[..3], raw[..3]);
        assert_eq!(out[4..n], raw[4..n], "only byte 3 and the digest change");
        assert_eq!(out[n..], sha256(&out[..n]));
        assert!(espflash_flash_image(&raw[..20]).is_err());
    }
}
