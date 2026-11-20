//! Espressif files the ROM tests fetch rather than commit.
//!
//! `scripts/fetch-esp-rom-elfs.sh` (or `.ps1`) downloads the pinned
//! esp-rom-elfs release and espflash's ESP32-C3 bootloader. The tests look
//! in `$OF_ESP_ROM_DIR`, then `$RUNNER_TEMP/esp-rom-elfs/<release>`, then
//! `.cache/esp-rom-elfs/<release>` in the repository, and skip (with a
//! note) when a file is absent, unless `OF_REQUIRE_ESP_ROM` is set, as in
//! CI. A file with the wrong SHA-256 always fails.

// Each test crate uses its own subset of these helpers.
#![allow(dead_code)]

use std::path::PathBuf;

use of_firmware::{ElfImage, sha256};
use of_soc_esp32c3::bootloader::{C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256};
use of_soc_esp32c3::rom::{C3_ROM_FILE, C3_ROM_SHA256, ROM_ELFS_RELEASE, RomImage};

/// The bytes of a fetched file, checked against its pinned SHA-256, or
/// `None` (after a note) when it has not been fetched.
pub fn fetched(file: &str, want_sha256: &str) -> Option<Vec<u8>> {
    let release = format!("esp-rom-elfs/{ROM_ELFS_RELEASE}");
    let dirs = [
        std::env::var_os("OF_ESP_ROM_DIR").map(PathBuf::from),
        std::env::var_os("RUNNER_TEMP").map(|t| PathBuf::from(t).join(&release)),
        Some(
            PathBuf::from(env!("CARGO_MANIFEST_DIR"))
                .join("../../.cache")
                .join(&release),
        ),
    ];
    let Some(path) = dirs.into_iter().flatten().map(|d| d.join(file)).find(|p| p.is_file()) else {
        assert!(
            std::env::var_os("OF_REQUIRE_ESP_ROM").is_none(),
            "OF_REQUIRE_ESP_ROM is set but {file} was not found; run scripts/fetch-esp-rom-elfs.sh"
        );
        eprintln!("skipped: {file} not found (run scripts/fetch-esp-rom-elfs.sh)");
        return None;
    };
    let bytes = std::fs::read(&path).expect("readable fetched file");
    let digest: String = sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    assert_eq!(digest, want_sha256, "{} is not the pinned file", path.display());
    Some(bytes)
}

/// The pinned ESP32-C3 revision 3 ROM.
pub fn rom() -> Option<RomImage> {
    let bytes = fetched(C3_ROM_FILE, C3_ROM_SHA256)?;
    let elf = ElfImage::parse(&bytes).expect("ROM ELF parses");
    Some(RomImage::from_elf(&elf).expect("ROM image builds"))
}

/// espflash's pinned ESP32-C3 bootloader, as fetched (before espflash
/// sets its flash header).
pub fn bootloader() -> Option<Vec<u8>> {
    fetched(C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256)
}
