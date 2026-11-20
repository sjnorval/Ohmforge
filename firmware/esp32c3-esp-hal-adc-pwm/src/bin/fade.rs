//! Ohmforge test firmware: an esp-hal LED fade in hardware.
//!
//! GPIO8 drives an LED from LEDC channel 0 on timer 0: 5 kHz from the
//! 80 MHz APB clock with 8-bit duty. The LEDC fades it up from 0 % to
//! 100 % and back down with esp-hal's `start_duty_fade` (100 ms asked for:
//! 500 PWM cycles over 255 steps, which esp-hal rounds down to one step
//! per cycle, so each fade takes 255 cycles, 51 ms), polling the fade-end
//! interrupt flag; each finished fade prints `fade: <n> up` or
//! `fade: <n> down` on UART0.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::DriveMode;
use esp_hal::ledc::channel::{self, ChannelIFace};
use esp_hal::ledc::timer::{self, TimerIFace};
use esp_hal::ledc::{LSGlobalClkSource, Ledc, LowSpeed};
use esp_hal::main;
use esp_hal::time::Rate;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

/// Asked-for fade duration (ms).
const FADE_MS: u16 = 100;

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    loop {}
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    println!("ohmforge esp-hal led fade");

    let mut ledc = Ledc::new(peripherals.LEDC);
    ledc.set_global_slow_clock(LSGlobalClkSource::APBClk);
    let mut timer0 = ledc.timer::<LowSpeed>(timer::Number::Timer0);
    timer0
        .configure(timer::config::Config {
            duty: timer::config::Duty::Duty8Bit,
            clock_source: timer::LSClockSource::APBClk,
            frequency: Rate::from_khz(5),
        })
        .unwrap();
    let mut led = ledc.channel(channel::Number::Channel0, peripherals.GPIO8);
    led.configure(channel::config::Config {
        timer: &timer0,
        duty_pct: 0,
        drive_mode: DriveMode::PushPull,
    })
    .unwrap();

    let mut n: u32 = 0;
    loop {
        // Start and end must differ: esp-hal divides by the duty change.
        for (from, to, dir) in [(0, 100, "up"), (100, 0, "down")] {
            led.start_duty_fade(from, to, FADE_MS).unwrap();
            while led.is_duty_fade_running() {}
            n += 1;
            println!("fade: {} {}", n, dir);
        }
    }
}
