//! The blinky firmware (firmware/esp32c3-blinky) on the bare SoC: ELF
//! direct boot, UART0 banner, IO MUX + GPIO matrix setup and a GPIO8
//! toggle timed by SYSTIMER.

use of_core::SimTime;
use of_cpu_riscv::RunExit;
use of_firmware::{ElfImage, Segment};
use of_soc_esp32c3::{Esp32c3, LoadError, PinOut};

const BLINKY: &[u8] = include_bytes!("../../../firmware/esp32c3-blinky/blinky.elf");

fn boot() -> (Esp32c3, ElfImage) {
    let img = ElfImage::parse(BLINKY).unwrap();
    let mut soc = Esp32c3::new();
    soc.load_elf(&img).unwrap();
    (soc, img)
}

fn word(soc: &Esp32c3, addr: u32) -> u32 {
    let b = soc.bus.peek(addr, 4).unwrap();
    u32::from_le_bytes([b[0], b[1], b[2], b[3]])
}

#[test]
fn elf_segments_land_in_flash_and_dram() {
    let (soc, img) = boot();
    assert_eq!(img.entry, 0x4200_0020);
    assert_eq!(soc.hart.pc(), img.entry);
    assert_eq!(img.segments.len(), 3, "text, rodata, data+bss");
    let banner = img.symbol("banner").map(|s| s.value);
    assert_eq!(banner, Some(0x3c01_0020));
    assert_eq!(soc.bus.peek(0x3c01_0020, 16).unwrap(), b"ohmforge blinky\n");
    assert_eq!(word(&soc, img.symbol("magic").unwrap().value), 0x0f0f_1234);
    assert_eq!(word(&soc, img.symbol("toggles").unwrap().value), 0, ".bss zeroed");
}

#[test]
fn segments_outside_the_map_are_rejected() {
    let mut soc = Esp32c3::new();
    let img = ElfImage {
        entry: 0,
        flags: 0,
        segments: vec![Segment {
            vaddr: 0x2000_0000,
            paddr: 0x2000_0000,
            data: vec![0; 4],
            mem_size: 4,
            flags: 4,
        }],
        symbols: vec![],
        sections: vec![],
    };
    assert_eq!(
        soc.load_elf(&img),
        Err(LoadError::Unmapped {
            addr: 0x2000_0000,
            len: 4
        })
    );
}

#[test]
fn gpio8_toggles_every_250_us_and_uart_prints_the_banner() {
    let (mut soc, img) = boot();
    assert_eq!(soc.run_until(SimTime::from_us(2_100)), RunExit::BudgetExhausted);
    assert_eq!(soc.uart_text(), "ohmforge blinky\n");

    let edges: Vec<_> = soc.pin_edges().iter().filter(|e| e.pin == 8).copied().collect();
    assert!(
        soc.pin_edges().iter().all(|e| e.pin == 8 || e.pin == 21),
        "only GPIO8 and U0TXD (GPIO21) change"
    );
    // Enable first drives the pin low, then the loop starts high.
    assert_eq!(edges[0].out, PinOut::driven(false, 2));
    let levels: Vec<_> = edges[1..].iter().map(|e| e.out.level.unwrap()).collect();
    assert!(levels.iter().step_by(2).all(|&l| l) && levels.iter().skip(1).step_by(2).all(|&l| !l));
    // Start-up (banner, setup) takes well under 10 µs.
    assert!(
        edges[1].at < SimTime::from_us(10),
        "first rising edge at {:?}",
        edges[1].at
    );
    // 2.1 ms covers 8 full half-periods after the first edge plus a bit.
    assert_eq!(edges.len() - 1, 9);
    for w in edges[1..].windows(2) {
        let dt = w[1].at.0 - w[0].at.0;
        // 250 µs, give or take one SYSTIMER poll (tens of cycles).
        assert!(dt.abs_diff(250_000_000) < 1_000_000, "half period {dt} ps");
    }
    // The firmware's own count of rising edges agrees.
    let toggles = img.symbol("toggles").unwrap().value;
    let rising = levels.iter().filter(|&&l| l).count() as u32;
    assert_eq!(word(&soc, toggles), rising);
    assert_eq!(soc.pin(8).level, Some(levels.len() % 2 == 1));
}

#[test]
fn deadlines_do_not_drift() {
    let (mut soc, _) = boot();
    soc.run_until(SimTime::from_ms(10));
    let edges: Vec<_> = soc.pin_edges().iter().filter(|e| e.pin == 8).skip(1).copied().collect();
    // Absolute deadlines: edge n is within one poll of first + n * 250 µs.
    let first = edges[0].at.0;
    for (n, e) in edges.iter().enumerate() {
        let ideal = first + n as u64 * 250_000_000;
        assert!(
            e.at.0.abs_diff(ideal) < 1_000_000,
            "edge {n} at {} ps, ideal {ideal}",
            e.at.0
        );
    }
    assert_eq!(edges.len(), 40);
}

#[test]
fn the_banner_leaves_on_u0txd_at_the_configured_baud() {
    let (mut soc, _) = boot();
    soc.bus.uart0.set_tx_line(true);
    // 16 frames at the reset baud (XTAL / 2 / 694, about 28.8 kBd; the
    // bare-metal blinky leaves UART0 unconfigured) take about 5.6 ms.
    soc.run_until(SimTime::from_ms(7));
    let baud = soc.bus.uart0.baud(of_soc_esp32c3::uart::APB_HZ).unwrap();
    let tx: Vec<(f64, bool)> = soc
        .pin_edges()
        .iter()
        .filter(|e| e.pin == 21)
        .map(|e| (e.at.as_secs_f64(), e.out.level.unwrap()))
        .collect();
    let level_at = |t: f64| tx.iter().take_while(|e| e.0 <= t).map(|e| e.1).last().unwrap_or(true);
    // Decode 8N1: each start bit's falling edge times the bit centres.
    let mut text = Vec::new();
    let mut i = 0;
    while let Some(&(start, _)) = tx[i..].iter().find(|e| !e.1) {
        let bit = |k: f64| level_at(start + (k + 0.5) / baud);
        assert!(!bit(0.0), "start bit");
        let byte = (0..8).fold(0u8, |b, k| b | (u8::from(bit(f64::from(k + 1))) << k));
        assert!(bit(9.0), "stop bit");
        text.push(byte);
        let end = start + 9.5 / baud;
        i = tx.iter().position(|e| e.0 > end).unwrap_or(tx.len());
    }
    assert_eq!(String::from_utf8(text).unwrap(), soc.uart_text());
}
