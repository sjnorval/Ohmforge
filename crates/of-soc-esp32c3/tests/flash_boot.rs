//! ROM-lite flash boot (`Esp32c3::boot_flash`): direct boot of the XIP
//! blinky, an app in the factory partition, an image at offset 0, and the
//! ways a flash can fail to boot.

use of_core::SimTime;
use of_cpu_riscv::{RunExit, asm::*};
use of_firmware::partition::{PartitionTable, TABLE_OFFSET};
use of_firmware::{ElfImage, EspImage, ImageError, ImageSegment};
use of_soc_esp32c3::{CHIP_ID, Esp32c3, FlashBoot, FlashBootError, map};

const XIP_ELF: &[u8] = include_bytes!("../../../firmware/esp32c3-xip-blinky/xip_blinky.elf");
const XIP_BIN: &[u8] = include_bytes!("../../../firmware/esp32c3-xip-blinky/xip_blinky.bin");
const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");

fn word(soc: &Esp32c3, addr: u32) -> u32 {
    let b = soc.bus.peek(addr, 4).expect("mapped");
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

fn gpio8_half_periods(soc: &Esp32c3) -> Vec<u64> {
    let edges: Vec<_> = soc.pin_edges().iter().filter(|e| e.pin == 8).collect();
    edges.windows(2).skip(1).map(|w| w[1].at.0 - w[0].at.0).collect()
}

/// 250 µs, give or take one SYSTIMER poll.
fn half_periods_ok(periods: &[u64]) -> bool {
    periods
        .iter()
        .all(|p| p.abs_diff(SimTime::from_us(250).0) < SimTime::from_us(1).0)
}

fn flash_mapped(addr: u32) -> bool {
    map::within(map::IROM_FLASH, addr, 1) || map::within(map::DROM_FLASH, addr, 1)
}

/// The blinky ELF as an app image laid out for flash offset `at`.
fn blinky_image(at: u32) -> Vec<u8> {
    let mut img = EspImage::from_elf(&ElfImage::parse(BLINKY).expect("elf"), CHIP_ID);
    img.align_flash_segments(at, flash_mapped).expect("fits");
    img.to_bytes()
}

#[test]
fn xip_blinky_direct_boots_from_flash() {
    let elf = ElfImage::parse(XIP_ELF).expect("elf");
    assert_eq!(elf.entry, 0x4200_0008);
    let mut soc = Esp32c3::new();
    soc.arm_boot_watchdogs();
    soc.write_flash(0, XIP_BIN);
    assert_eq!(soc.boot_flash(), Ok(FlashBoot::Direct));
    assert_eq!(soc.hart.pc(), 0x4200_0008);
    assert_eq!(soc.run_until(SimTime::from_us(3_100)), RunExit::BudgetExhausted);
    assert_eq!(soc.uart_text(), "ohmforge xip blinky\n");
    let sym = |n: &str| elf.symbol(n).expect(n).value;
    assert_eq!(
        word(&soc, sym("drom_sum")),
        120 + 0x1_0000,
        "DROM reads see the flash bytes"
    );
    // `toggles` counts rising edges (one per 500 µs period).
    let rising = soc
        .pin_edges()
        .iter()
        .filter(|e| e.pin == 8 && e.out.level == Some(true))
        .count() as u32;
    assert_eq!(word(&soc, sym("toggles")), rising);
    assert!((6..=7).contains(&rising), "rising edges {rising}");
    let periods = gpio8_half_periods(&soc);
    assert!(periods.len() >= 10);
    assert!(half_periods_ok(&periods), "{periods:?}");
    assert!(soc.resets().is_empty(), "the firmware disables the boot watchdogs");
}

#[test]
fn factory_app_boots_from_the_partition_table() {
    let mut soc = Esp32c3::new();
    soc.write_flash(TABLE_OFFSET, &PartitionTable::single_app().to_bytes());
    soc.write_flash(0x1_0000, &blinky_image(0x1_0000));
    let booted = soc.boot_flash().expect("boots");
    assert_eq!(
        booted,
        FlashBoot::Partition {
            label: "factory".into(),
            offset: 0x1_0000,
            entry: 0x4200_0020
        }
    );
    // The window shows the app's flash bytes at its link addresses.
    assert_eq!(soc.bus.mmu_lookup(0x4200_0020).map(|f| f % 0x1_0000), Some(0x20));
    assert!(soc.bus.mmu_lookup(0x4200_0020).expect("mapped") >= 0x1_0000);
    assert_eq!(soc.bus.peek(0x3c01_0020, 16), Some(&b"ohmforge blinky\n"[..]));
    soc.run_until(SimTime::from_us(2_100));
    assert_eq!(soc.uart_text(), "ohmforge blinky\n");
    let periods = gpio8_half_periods(&soc);
    assert!(periods.len() >= 6 && half_periods_ok(&periods), "{periods:?}");
}

#[test]
fn image_at_zero_boots_without_a_table() {
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &blinky_image(0));
    assert_eq!(soc.boot_flash(), Ok(FlashBoot::ImageAtZero { entry: 0x4200_0020 }));
    soc.run_until(SimTime::from_us(100));
    assert_eq!(soc.uart_text(), "ohmforge blinky\n");
}

#[test]
fn watchdog_reset_boots_the_flash_again() {
    // A RAM-only app that prints '!' and sleeps without touching the boot
    // watchdogs: MWDT0 resets it every 325 ms and ROM-lite boots the
    // partition app again each time.
    const BASE: u32 = 0x4038_0000;
    let mut prog = li(5, 0x6000_0000).to_vec();
    prog.extend([addi(6, 0, i32::from(b'!')), sw(6, 5, 0), WFI, jal(0, -4)]);
    let mut img = EspImage::new(CHIP_ID, BASE);
    img.segments = vec![ImageSegment {
        addr: BASE,
        data: prog.iter().flat_map(|w| w.to_le_bytes()).collect(),
    }];
    let mut soc = Esp32c3::new();
    soc.write_flash(TABLE_OFFSET, &PartitionTable::single_app().to_bytes());
    soc.write_flash(0x1_0000, &img.to_bytes());
    soc.arm_boot_watchdogs();
    assert_eq!(soc.boot_flash().map(|b| b.entry()), Ok(BASE));
    soc.run_until(SimTime::from_ms(700));
    let at: Vec<_> = soc.resets().iter().map(|r| r.at).collect();
    assert_eq!(at, [SimTime::from_ms(325), SimTime::from_ms(650)]);
    assert_eq!(soc.uart_text(), "!!!");
}

#[test]
fn unbootable_flash_is_reported() {
    let mut soc = Esp32c3::new();
    assert_eq!(soc.boot_flash(), Err(FlashBootError::Nothing(0xffff_ffff)));

    // An app image that ignores the 64 KiB alignment rule.
    let mut soc = Esp32c3::new();
    let img = EspImage::from_elf(&ElfImage::parse(BLINKY).expect("elf"), CHIP_ID);
    soc.write_flash(TABLE_OFFSET, &PartitionTable::single_app().to_bytes());
    soc.write_flash(0x1_0000, &img.to_bytes());
    assert!(matches!(soc.boot_flash(), Err(FlashBootError::Misaligned { .. })));

    // A corrupt image.
    let mut bytes = blinky_image(0x1_0000);
    bytes[40] ^= 0xff;
    soc.write_flash(0x1_0000, &bytes);
    assert!(matches!(
        soc.boot_flash(),
        Err(FlashBootError::Image {
            offset: 0x1_0000,
            error: ImageError::Checksum { .. }
        })
    ));

    // An image for another chip.
    let mut other = EspImage::from_elf(&ElfImage::parse(BLINKY).expect("elf"), 0);
    other.align_flash_segments(0, flash_mapped).expect("fits");
    let mut soc = Esp32c3::new();
    soc.write_flash(0, &other.to_bytes());
    assert_eq!(soc.boot_flash(), Err(FlashBootError::WrongChip(0)));
}
