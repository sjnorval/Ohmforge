//! Locate the fetched ESP32-C3 ROM and espflash bootloader.
//!
//! The pot dimmer, LED fade and push-button demos boot the committed esp-hal images the
//! same way the integration tests do: the real mask ROM, then espflash's
//! ESP-IDF second-stage bootloader, then the app. Those files are never
//! bundled; `scripts/fetch-esp-rom-elfs.sh` (or `.ps1`) downloads a pinned,
//! checksummed copy.
//!
//! App-only loads (`Esp32c3::load_elf` / `load_flash_image`) and ROM-lite
//! (`boot_flash`) were tried for these images so the desktop app could run
//! without a fetch, the way the bare-metal blinky demo does. Neither path
//! starts the esp-hal apps: the hart spins early in `esp_hal::init` with no
//! UART output. The GUI therefore requires the fetched files and explains
//! how to get them when they are missing.

use of_circuit::fs::{FetchError, fetch_pinned};
use of_firmware::ElfImage;
use of_soc_esp32c3::Esp32c3;
use of_soc_esp32c3::bootloader::{C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256};
use of_soc_esp32c3::rom::{C3_ROM_FILE, C3_ROM_SHA256, ROM_ELFS_RELEASE, RomImage, boot_from_rom};

/// Shown in the GUI when the ROM or bootloader has not been fetched.
pub(crate) const MISSING_MESSAGE: &str = "This demo needs Espressif's ESP32-C3 ROM and espflash's second-stage bootloader, which are not bundled with Ohmforge. Run scripts/fetch-esp-rom-elfs.ps1 (or scripts/fetch-esp-rom-elfs.sh) and click Reset. Unlike the LED blink demo, these esp-hal apps (dimmer, fade, and the push-button demos) cannot start from the app image alone.";

/// The search folders and checks are `of_circuit::fs::fetch_pinned`'s
/// (`OF_ESP_ROM_DIR`, `RUNNER_TEMP`, `.cache`; `OF_REQUIRE_ESP_ROM` panics).
fn fetched(file: &str, want_sha256: &str) -> Result<Vec<u8>, String> {
    fetch_pinned(ROM_ELFS_RELEASE, file, want_sha256).map_err(|e| match e {
        FetchError::Missing(_) => MISSING_MESSAGE.to_owned(),
        FetchError::Invalid(msg) => msg,
    })
}

fn rom_and_bootloader() -> Result<(RomImage, Vec<u8>), String> {
    let rom_bytes = fetched(C3_ROM_FILE, C3_ROM_SHA256)?;
    let elf = ElfImage::parse(&rom_bytes).map_err(|e| format!("ROM ELF did not parse: {e}"))?;
    let rom = RomImage::from_elf(&elf).map_err(|e| format!("ROM image did not build: {e}"))?;
    let bootloader = fetched(C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256)?;
    Ok((rom, bootloader))
}

/// Power on the fetched ROM and boot `app` from flash until `banner` appears.
pub(crate) fn boot_app(app: &[u8], partitions: &[u8], banner: &str) -> Result<Esp32c3, String> {
    let (rom, bootloader) = rom_and_bootloader()?;
    boot_from_rom(&rom, &bootloader, app, partitions, banner)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_message_points_at_the_fetch_scripts() {
        assert!(MISSING_MESSAGE.contains("scripts/fetch-esp-rom-elfs.ps1"));
        assert!(MISSING_MESSAGE.contains("scripts/fetch-esp-rom-elfs.sh"));
        assert!(MISSING_MESSAGE.contains("not bundled"));
    }
}
