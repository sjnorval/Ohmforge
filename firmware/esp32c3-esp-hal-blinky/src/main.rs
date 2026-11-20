//! Ohmforge test firmware: an unmodified esp-hal blinky for the ESP32-C3.
//!
//! Built the normal way (esp-hal, ESP-IDF app descriptor) and flashed with
//! `espflash save-image --merge`, so the simulator runs the full boot path:
//! ROM, the ESP-IDF second-stage bootloader, then this app from flash.
//! It prints a banner on UART0 and toggles GPIO8 every 500 µs.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Level, Output, OutputConfig};
use esp_hal::main;
use esp_hal::time::{Duration, Instant};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

/// Half period: GPIO8 toggles this often.
const HALF_PERIOD: Duration = Duration::from_micros(500);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    loop {}
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    println!("ohmforge esp-hal blinky");

    let mut led = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    let mut next = Instant::now();
    loop {
        led.toggle();
        next += HALF_PERIOD;
        while Instant::now() < next {}
    }
}
