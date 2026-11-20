//! Locate the fetched ESP32-C3 ROM and espflash bootloader.
//!
//! The pot dimmer and LED fade demos boot the committed esp-hal images the
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

use std::path::PathBuf;

use of_firmware::{ElfImage, sha256};
use of_soc_esp32c3::Esp32c3;
use of_soc_esp32c3::bootloader::{C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256, espflash_flash_image};
use of_soc_esp32c3::rom::{C3_ROM_FILE, C3_ROM_SHA256, ROM_ELFS_RELEASE, RomImage};

use of_core::SimTime;

/// Shown in the GUI when the ROM or bootloader has not been fetched.
pub(crate) const MISSING_MESSAGE: &str = "This demo needs Espressif's ESP32-C3 ROM and espflash's second-stage bootloader, which are not bundled with Ohmforge. Run scripts/fetch-esp-rom-elfs.ps1 (or scripts/fetch-esp-rom-elfs.sh) and click Reset. Unlike the LED blink demo, these esp-hal apps cannot start from the app image alone.";

fn cache_dirs() -> Vec<PathBuf> {
    let release = format!("esp-rom-elfs/{ROM_ELFS_RELEASE}");
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("OF_ESP_ROM_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(temp) = std::env::var_os("RUNNER_TEMP") {
        dirs.push(PathBuf::from(temp).join(&release));
    }
    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache")
            .join(&release),
    );
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join(".cache").join(&release));
    }
    dirs
}

fn fetched(file: &str, want_sha256: &str) -> Result<Vec<u8>, String> {
    let Some(path) = cache_dirs().into_iter().map(|d| d.join(file)).find(|p| p.is_file()) else {
        if std::env::var_os("OF_REQUIRE_ESP_ROM").is_some() {
            panic!("OF_REQUIRE_ESP_ROM is set but {file} was not found; run scripts/fetch-esp-rom-elfs.sh");
        }
        return Err(MISSING_MESSAGE.to_owned());
    };
    let bytes = std::fs::read(&path).map_err(|e| format!("could not read {}: {e}", path.display()))?;
    let digest: String = sha256(&bytes).iter().map(|b| format!("{b:02x}")).collect();
    if digest != want_sha256 {
        return Err(format!(
            "{} is not the pinned file (SHA-256 {digest}, expected {want_sha256})",
            path.display()
        ));
    }
    Ok(bytes)
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
    let mut soc = Esp32c3::new();
    let image = espflash_flash_image(&bootloader).map_err(|e| format!("bootloader image: {e}"))?;
    soc.write_flash(0, &image);
    soc.write_flash(0x8000, partitions);
    soc.write_flash(0x1_0000, app);
    soc.power_on_rom(&rom);
    let mut t = SimTime::ZERO;
    while !soc.uart_text().contains(banner) {
        if t >= SimTime::from_ms(100) {
            return Err(format!("firmware did not print its banner: {:?}", soc.uart_text()));
        }
        t = t.saturating_add(SimTime::from_ms(1));
        soc.run_until(t);
    }
    if !soc.resets().is_empty() {
        return Err(format!("chip reset during boot: {:?}", soc.resets()));
    }
    Ok(soc)
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
