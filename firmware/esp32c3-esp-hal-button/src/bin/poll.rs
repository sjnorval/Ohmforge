//! Ohmforge test firmware: an esp-hal push button, polled.
//!
//! GPIO9 is the button input (pressed = low; the internal pull-up is on and
//! the board may add its own), GPIO8 drives the LED. The loop copies the
//! button to the LED as fast as it can, with no debouncing, so every
//! contact bounce shows on the LED. Each change is counted and printed on
//! UART0 as `poll: <count> <level>`.

#![no_std]
#![no_main]

use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Input, InputConfig, Level, Output, OutputConfig, Pull};
use esp_hal::main;
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    loop {}
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let button = Input::new(peripherals.GPIO9, InputConfig::default().with_pull(Pull::Up));
    let mut led = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    println!("ohmforge esp-hal button (polling)");

    let mut last = button.level();
    let mut changes: u32 = 0;
    loop {
        let level = button.level();
        // LED on while the button is pressed (pulled low).
        led.set_level(if level == Level::Low { Level::High } else { Level::Low });
        if level != last {
            last = level;
            changes += 1;
            println!("poll: {} {}", changes, if level == Level::Low { "low" } else { "high" });
        }
    }
}
