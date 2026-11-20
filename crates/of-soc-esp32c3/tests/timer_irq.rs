//! The timer-interrupt firmware (firmware/esp32c3-timer-irq) on the bare
//! SoC: SYSTIMER and TIMG0 alarms through the interrupt matrix into
//! vectored traps, watchdogs switched off by firmware, and watchdog resets
//! for firmware that does not.

use of_core::SimTime;
use of_cpu_riscv::{RunExit, asm::*};
use of_firmware::{ElfImage, EspImage};
use of_soc_esp32c3::{CHIP_ID, Esp32c3, LoadError, ResetKind, ResetSource};

const TIMER_IRQ: &[u8] = include_bytes!("../../../firmware/esp32c3-timer-irq/timer_irq.elf");

fn boot() -> (Esp32c3, ElfImage) {
    let img = ElfImage::parse(TIMER_IRQ).unwrap();
    let mut soc = Esp32c3::new();
    soc.load_elf(&img).unwrap();
    (soc, img)
}

fn var(soc: &Esp32c3, img: &ElfImage, name: &str) -> u32 {
    let b = soc.bus.peek(img.symbol(name).unwrap().value, 4).unwrap();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[test]
fn systimer_period_interrupt_toggles_gpio8_without_drift() {
    let (mut soc, img) = boot();
    assert_eq!(soc.run_until(SimTime::from_ms(10)), RunExit::WaitingForInterrupt);
    assert_eq!(soc.uart_text(), "ohmforge timer irq\n");
    assert_eq!(var(&soc, &img, "fault"), 0, "no unexpected trap");

    let edges: Vec<_> = soc.pin_edges().iter().filter(|e| e.pin == 8).copied().collect();
    // Enabling the output drives it low; then every interrupt toggles it.
    assert_eq!(edges[0].out.level, Some(false));
    let toggles = &edges[1..];
    let first = toggles[0].at.0;
    // The comparator was armed during start-up (a few µs in).
    assert!(
        (250_000_000..260_000_000).contains(&first),
        "first toggle at {first} ps"
    );
    for (n, e) in toggles.iter().enumerate() {
        assert_eq!(e.out.level, Some(n % 2 == 0), "toggle {n}");
        // Period mode: every alarm on the 250 µs grid; the handler's GPIO
        // write comes a few instructions after the interrupt (within 1 µs,
        // allowing for a TIMG0 handler running first).
        let ideal = first + n as u64 * 250_000_000;
        assert!(
            e.at.0.abs_diff(ideal) < 1_000_000,
            "toggle {n} at {} ps, ideal {ideal}",
            e.at.0
        );
    }
    assert_eq!(toggles.len(), 39, "(10 ms - start-up) / 250 µs");
    assert_eq!(var(&soc, &img, "sys_ticks"), 39);
}

#[test]
fn timg0_alarm_interrupt_counts_milliseconds() {
    let (mut soc, img) = boot();
    soc.run_until(SimTime::from_ms(20));
    // Armed a few µs after reset: alarms at ~1 ms, 2 ms, ... 19 ms.
    assert_eq!(var(&soc, &img, "tg_ticks"), 19);
    soc.run_until(SimTime::from_ms(1000));
    assert_eq!(var(&soc, &img, "tg_ticks"), 999, "auto-reload alarms do not drift");
    assert_eq!(var(&soc, &img, "sys_ticks"), 3999);
    assert_eq!(var(&soc, &img, "fault"), 0);
}

#[test]
fn the_hart_sleeps_between_interrupts() {
    let (mut soc, _) = boot();
    soc.run_until(SimTime::from_ms(10));
    assert!(soc.is_sleeping());
    let instret = soc.hart.csrs().minstret;
    let cycles = soc.hart.elapsed_cycles();
    assert_eq!(cycles, 1_600_000, "160 MHz for 10 ms");
    // Start-up plus 50 short handlers: a few thousand instructions.
    assert!(instret < 3_000, "{instret} instructions retired");
}

#[test]
fn firmware_disables_the_boot_watchdogs() {
    let (mut soc, img) = boot();
    soc.arm_boot_watchdogs();
    soc.run_until(SimTime::from_ms(2_500));
    assert_eq!(soc.resets(), &[], "MWDT0, RWDT and SWD all handled");
    assert_eq!(var(&soc, &img, "tg_ticks"), 2_499);
}

#[test]
fn flash_image_boots_like_the_elf() {
    let elf = ElfImage::parse(TIMER_IRQ).unwrap();
    let bin = EspImage::from_elf(&elf, CHIP_ID).to_bytes();
    let img = EspImage::parse(&bin).unwrap();
    let mut soc = Esp32c3::new();
    soc.load_flash_image(&img).unwrap();
    soc.run_until(SimTime::from_ms(5));
    assert_eq!(soc.uart_text(), "ohmforge timer irq\n");
    assert_eq!(var(&soc, &elf, "tg_ticks"), 4);

    let other = EspImage::from_elf(&elf, 9); // ESP32-S3
    assert_eq!(Esp32c3::new().load_flash_image(&other), Err(LoadError::WrongChip(9)));
}

/// Print one byte, then sleep forever with interrupts off; optionally
/// switch off MWDT0 first.
fn sleeper(disable_mwdt0: bool) -> Esp32c3 {
    const BASE: u32 = 0x4038_0000;
    let mut prog = Vec::new();
    prog.extend(li(5, 0x6000_0000));
    prog.extend([addi(6, 0, i32::from(b'!')), sw(6, 5, 0)]);
    if disable_mwdt0 {
        prog.extend(li(5, 0x6001_f000));
        prog.extend(li(6, 0x50d8_3aa1));
        prog.extend([sw(6, 5, 0x64), sw(0, 5, 0x48), sw(0, 5, 0x64)]);
    }
    prog.extend([WFI, jal(0, -4)]);
    let bytes: Vec<u8> = prog.iter().flat_map(|w| w.to_le_bytes()).collect();
    let img = ElfImage {
        entry: BASE,
        flags: 0,
        segments: vec![of_firmware::Segment {
            vaddr: BASE,
            paddr: BASE,
            mem_size: bytes.len() as u32,
            data: bytes,
            flags: 5,
        }],
        symbols: vec![],
        sections: vec![],
    };
    let mut soc = Esp32c3::new();
    soc.load_elf(&img).unwrap();
    soc.arm_boot_watchdogs();
    soc
}

#[test]
fn mwdt0_flash_boot_protection_resets_and_reboots() {
    let mut soc = sleeper(false);
    soc.run_until(SimTime::from_ms(1_000));
    let at: Vec<_> = soc.resets().iter().map(|r| r.at).collect();
    assert_eq!(
        at,
        [SimTime::from_ms(325), SimTime::from_ms(650), SimTime::from_ms(975)]
    );
    assert!(
        soc.resets()
            .iter()
            .all(|r| r.source == ResetSource::Mwdt(0) && r.kind == ResetKind::System)
    );
    // Each reset boots the firmware again: one '!' per boot.
    assert_eq!(soc.uart_text(), "!!!!");
    assert_eq!(soc.uart_bytes()[1].at.0 / 1_000_000_000, 325);
}

#[test]
fn super_watchdog_fires_when_only_mwdt0_is_disabled() {
    let mut soc = sleeper(true);
    soc.run_until(SimTime::from_ms(1_200));
    let r = soc.resets();
    assert_eq!(r.len(), 1);
    assert_eq!(
        (r[0].at, r[0].source, r[0].kind),
        (SimTime::from_ms(1_000), ResetSource::Swd, ResetKind::System)
    );
}
