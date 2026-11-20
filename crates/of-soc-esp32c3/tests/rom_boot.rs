//! The real ESP32-C3 mask ROM (revision 3) booting on the model.
//!
//! The ROM ELF is Espressif's (esp-rom-elfs, Apache-2.0, closed-source
//! code) and is never committed. Fetch it with
//! `scripts/fetch-esp-rom-elfs.sh` (or `.ps1`); see `common/mod.rs` for
//! where the tests look and when they skip.

mod common;

use common::rom;
use of_core::SimTime;
use of_cpu_riscv::asm::*;
use of_firmware::{ElfImage, EspImage, ImageSegment};
use of_soc_esp32c3::bus::STRAP_DOWNLOAD;
use of_soc_esp32c3::{CHIP_ID, Esp32c3, ResetKind, ResetSource};

const XIP_ELF: &[u8] = include_bytes!("../../../firmware/esp32c3-xip-blinky/xip_blinky.elf");
const XIP_BIN: &[u8] = include_bytes!("../../../firmware/esp32c3-xip-blinky/xip_blinky.bin");

const BANNER: &str = "ESP-ROM:esp32c3-api1-20210207\r\nBuild:Feb  7 2021\r\n";
const POWER_ON: &str = "rst:0x1 (POWERON),boot:0xc (SPI_FAST_FLASH_BOOT)\r\n";

/// A RAM-only app image: print '!' and sleep, never touching a watchdog.
fn ram_image() -> EspImage {
    const BASE: u32 = 0x4038_0000;
    let mut prog = li(5, 0x6000_0000).to_vec();
    prog.extend([addi(6, 0, i32::from(b'!')), sw(6, 5, 0), WFI, jal(0, -4)]);
    let mut img = EspImage::new(CHIP_ID, BASE);
    img.segments = vec![ImageSegment {
        addr: BASE,
        data: prog.iter().flat_map(|w| w.to_le_bytes()).collect(),
    }];
    img
}

#[test]
fn blank_flash_prints_the_banner_and_invalid_header() {
    let Some(rom) = rom() else { return };
    let mut soc = Esp32c3::new();
    soc.power_on_rom(&rom);
    soc.run_until(SimTime::from_ms(2));
    let text = soc.uart_text();
    assert!(
        text.starts_with(&format!("{BANNER}{POWER_ON}invalid header: 0xffffffff\r\n")),
        "{text:?}"
    );
    // The ROM prints on the USB Serial/JTAG console too.
    assert!(String::from_utf8_lossy(soc.bus.usb_serial.tx()).starts_with(BANNER));
}

#[test]
fn rom_direct_boots_the_xip_blinky() {
    let Some(rom) = rom() else { return };
    let elf = ElfImage::parse(XIP_ELF).expect("elf");
    let mut soc = Esp32c3::new();
    soc.write_flash(0, XIP_BIN);
    soc.power_on_rom(&rom);
    soc.run_until(SimTime::from_ms(4));
    assert_eq!(soc.uart_text(), format!("{BANNER}{POWER_ON}ohmforge xip blinky\n"));
    // ets_run_direct_boot mapped flash one to one and enabled the cache.
    assert!((0..64).all(|n| soc.bus.cache.entry(n) == n as u32));
    assert!(soc.hart.pc() >= 0x4200_0000 && soc.hart.pc() < 0x4200_1000);
    let sum = elf.symbol("drom_sum").expect("drom_sum").value;
    let b = soc.bus.peek(sum, 4).expect("mapped");
    assert_eq!(u32::from_le_bytes([b[0], b[1], b[2], b[3]]), 120 + 0x1_0000);
    let edges: Vec<_> = soc.pin_edges().iter().filter(|e| e.pin == 8).collect();
    assert!(edges.len() >= 10, "{} GPIO8 edges", edges.len());
    for w in edges[1..].windows(2) {
        let dt = w[1].at.0 - w[0].at.0;
        assert!(
            dt.abs_diff(SimTime::from_us(250).0) < SimTime::from_us(1).0,
            "half period {dt} ps"
        );
    }
    assert!(soc.resets().is_empty());
}

#[test]
fn rom_loads_and_verifies_an_image_at_offset_0() {
    let Some(rom) = rom() else { return };
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &ram_image().to_bytes());
    soc.power_on_rom(&rom);
    soc.run_until(SimTime::from_ms(5));
    let text = soc.uart_text();
    // The ROM's loader read the header, loaded the segment, checked the
    // checksum and the SHA-256 (on the SHA accelerator) and jumped.
    assert_eq!(
        text,
        format!(
            "{BANNER}{POWER_ON}SPIWP:0xee\r\nmode:DIO, clock div:2\r\nload:0x40380000,len:0x18\r\nentry 0x40380000\r\n!"
        )
    );
    assert!(soc.bus.sha.blocks > 0);
    assert!(soc.is_sleeping());
}

#[test]
fn rom_reports_a_bad_digest_and_boots_anyway() {
    let Some(rom) = rom() else { return };
    let mut bytes = ram_image().to_bytes();
    let last = bytes.len() - 1;
    bytes[last] ^= 0xff;
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &bytes);
    soc.power_on_rom(&rom);
    soc.run_until(SimTime::from_ms(10));
    let text = soc.uart_text();
    assert!(text.contains("SHA-256 comparison failed:\r\n"), "{text:?}");
    assert!(
        text.ends_with("Attempting to boot anyway...\r\nentry 0x40380000\r\n!"),
        "{text:?}"
    );
}

#[test]
fn mwdt0_flash_boot_protection_resets_into_the_rom() {
    let Some(rom) = rom() else { return };
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &ram_image().to_bytes());
    soc.power_on_rom(&rom);
    soc.run_until(SimTime::from_ms(400));
    let r = soc.resets();
    assert_eq!(r.len(), 1);
    assert_eq!((r[0].source, r[0].kind), (ResetSource::Mwdt(0), ResetKind::System));
    let text = soc.uart_text();
    // The second boot reports the watchdog as the reset cause.
    assert!(
        text.contains("!ESP-ROM:esp32c3-api1-20210207\r\nBuild:Feb  7 2021\r\nrst:0x7 (TG0WDT_SYS_RST),boot:0xc (SPI_FAST_FLASH_BOOT)\r\n"),
        "{text:?}"
    );
    assert!(text.ends_with("entry 0x40380000\r\n!"), "{text:?}");
}

#[test]
fn download_strapping_enters_the_serial_bootloader() {
    let Some(rom) = rom() else { return };
    let mut soc = Esp32c3::new();
    soc.bus.strapping = STRAP_DOWNLOAD;
    soc.write_flash(0, XIP_BIN);
    soc.power_on_rom(&rom);
    soc.run_until(SimTime::from_ms(2));
    let text = soc.uart_text();
    // The flash is not booted.
    assert_eq!(
        text,
        format!("{BANNER}rst:0x1 (POWERON),boot:0x4 (DOWNLOAD(USB/UART0/1))\r\nwaiting for download\r\n")
    );
}
