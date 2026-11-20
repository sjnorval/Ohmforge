//! Ohmforge test firmware: an esp-hal push button on a GPIO interrupt.
//!
//! GPIO9 is the button input (pressed = low; the internal pull-up is on),
//! GPIO8 drives the LED. Every falling edge on GPIO9 raises the GPIO
//! interrupt; the handler toggles the LED and counts the press. There is
//! no debouncing, so a bouncing contact toggles the LED several times. The
//! main loop sleeps in `wfi` and prints `irq: <count>` when the count
//! changes.

#![no_std]
#![no_main]

use core::cell::RefCell;
use core::sync::atomic::{AtomicU32, Ordering};

use critical_section::Mutex;
use esp_hal::clock::CpuClock;
use esp_hal::gpio::{Event, Input, InputConfig, Io, Level, Output, OutputConfig, Pull};
use esp_hal::{handler, main};
use esp_println::println;

esp_bootloader_esp_idf::esp_app_desc!();

static BUTTON: Mutex<RefCell<Option<Input<'static>>>> = Mutex::new(RefCell::new(None));
static LED: Mutex<RefCell<Option<Output<'static>>>> = Mutex::new(RefCell::new(None));
static PRESSES: AtomicU32 = AtomicU32::new(0);

#[panic_handler]
fn panic(info: &core::panic::PanicInfo) -> ! {
    println!("panic: {}", info);
    loop {}
}

#[handler]
fn on_gpio() {
    critical_section::with(|cs| {
        if let Some(button) = BUTTON.borrow_ref_mut(cs).as_mut()
            && button.is_interrupt_set()
        {
            button.clear_interrupt();
            if let Some(led) = LED.borrow_ref_mut(cs).as_mut() {
                led.toggle();
            }
            // No atomic read-modify-write on RV32IMC; this runs in a
            // critical section and is the only writer.
            PRESSES.store(PRESSES.load(Ordering::Relaxed) + 1, Ordering::Relaxed);
        }
    });
}

#[main]
fn main() -> ! {
    let peripherals = esp_hal::init(esp_hal::Config::default().with_cpu_clock(CpuClock::max()));
    let mut io = Io::new(peripherals.IO_MUX);
    io.set_interrupt_handler(on_gpio);
    let mut button = Input::new(peripherals.GPIO9, InputConfig::default().with_pull(Pull::Up));
    let led = Output::new(peripherals.GPIO8, Level::Low, OutputConfig::default());
    critical_section::with(|cs| {
        button.listen(Event::FallingEdge);
        BUTTON.borrow_ref_mut(cs).replace(button);
        LED.borrow_ref_mut(cs).replace(led);
    });
    println!("ohmforge esp-hal button (interrupt)");

    let mut seen = 0;
    loop {
        let n = PRESSES.load(Ordering::Relaxed);
        if n != seen {
            seen = n;
            println!("irq: {}", n);
        }
        // Sleep until the next interrupt.
        unsafe { core::arch::asm!("wfi") };
    }
}
