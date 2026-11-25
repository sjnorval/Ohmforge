//! ESP32-C3 SoC: memory map, peripherals, flash and cache, boot (real ROM or
//! ROM-lite) and the kernel engine.
//!
//! [`Esp32c3`] wires an [`of_cpu_riscv::Hart`] to [`SocBus`], the C3 memory
//! map (ROM, SRAM through its IRAM and DRAM windows, the flash cache windows
//! and RTC fast memory) and its peripherals: GPIO with a minimal IO MUX and
//! GPIO matrix, UART0 transmit, SYSTIMER (counters and alarms), timer groups
//! TIMG0/1 (timer and MWDT), the RTC and super watchdogs, and the interrupt
//! matrix with the CPU interrupt controller, the LED PWM controller
//! ([`ledc`], exact switching edges on the pins) and the SAR ADC's one-time
//! sampling ([`saradc`], pad voltages sampled from the analog engine,
//! eFuse-calibrated transfer). Timer, watchdog, PWM and ADC events stop
//! the hart at their exact time, so interrupts and resets land where they
//! would on hardware (to within one instruction).
//! [`Esp32c3Engine`] runs it under the [`of_core::Kernel`] and turns pin
//! changes into events for the analog engine.
//!
//! Flash: a 4 MiB SPI NOR chip ([`flash::SpiFlash`]) behind the SPI0/SPI1
//! memory controllers ([`spi_mem`]), and the cache MMU ([`cache`]) that maps
//! 64 KiB flash pages into the IROM (`0x4200_0000`) and DROM
//! (`0x3C00_0000`) windows. Cache lines and miss timing are not modelled:
//! window accesses cost what SRAM accesses cost.
//!
//! Boot, three ways:
//! - **Real ROM**: [`Esp32c3::power_on_rom`] runs Espressif's mask ROM
//!   ([`rom::RomImage`], from esp-rom-elfs, fetched, never committed) from
//!   the reset vector. It reads the strapping pins, the reset cause and the
//!   flash, and direct boots or loads an image at flash offset 0, as on
//!   hardware. Support peripherals for it: [`csr`] (performance counters),
//!   [`efuse`], [`sha`] (image digests) and [`usb_serial`] (console).
//! - **ROM-lite**: [`Esp32c3::boot_flash`] does in zero time what the ROM
//!   and the second-stage bootloader would leave behind: direct boot, the
//!   app from the partition table at `0x8000`, or the image at 0
//!   ([`boot`]).
//! - **Direct load**: [`Esp32c3::load_elf`] or
//!   [`Esp32c3::load_flash_image`] place an image in memory and jump to its
//!   entry, with flash-window segments at the matching flash offsets.
//!
//! Register addresses follow the ESP32-C3 Technical Reference Manual. Reset
//! values and behaviour of anything not listed in the module docs are
//! approximations: unmodelled peripheral registers read back what was last
//! written and are counted in [`SocBus::unmodelled`].

pub mod boot;
pub mod bootloader;
pub mod bus;
pub mod cache;
pub mod component;
pub mod csr;
pub mod efuse;
pub mod engine;
pub mod flash;
pub mod gpio;
pub mod intmatrix;
pub mod ledc;
pub mod map;
pub mod pad;
pub mod rng;
pub mod rom;
pub mod rtc_cntl;
pub mod saradc;
pub mod sha;
pub mod soc;
pub mod spi_mem;
pub mod system;
pub mod systimer;
pub mod timg;
pub mod uart;
pub mod usb_serial;
pub mod watchdog;

pub use boot::{FlashBoot, FlashBootError};
pub use bus::SocBus;
pub use component::{BootImage, Esp32c3Component, QUIESCENT_A, QUIESCENT_WFI_A, UART0_PORT, register};
pub use engine::{
    Esp32c3Engine, InputWatch, PinLink, adc_request_code, adc_value_code, all_pin_codes, decode_adc_request,
    decode_input_code, decode_pin_code, initial_input_level, input_code, input_watches, pin_code,
};
pub use gpio::{GPIO_COUNT, PinOut, VIH_FRACTION, VIL_FRACTION};
pub use intmatrix::IntMatrix;
pub use pad::{add_pin_drivers, hook_pin, pin_actions, pin_driver_names, pin_legs};
pub use soc::{CHIP_ID, Esp32c3, LoadError, PinEdge, UartByte};
pub use watchdog::{ResetKind, ResetSource, WatchdogReset};
