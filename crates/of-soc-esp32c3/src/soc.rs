//! The ESP32-C3 as a whole: hart, bus and time-stamped outputs.

use of_core::SimTime;
use of_cpu_riscv::{BusError, Hart, HartConfig, HartState, RunExit};
use of_firmware::{ElfImage, EspImage};
use thiserror::Error;

use crate::boot::{self, FlashBoot, FlashBootError};
use crate::bus::SocBus;
use crate::gpio::{GPIO_COUNT, PinOut};
use crate::map::{self, within};
use crate::rom::RomImage;
use crate::watchdog::{ResetKind, WatchdogReset};

/// A pin output change, stamped with the hart time of the instruction
/// that caused it (the end of that instruction).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PinEdge {
    /// When.
    pub at: SimTime,
    /// GPIO number.
    pub pin: u8,
    /// New output state.
    pub out: PinOut,
}

/// A byte written to UART0, stamped like [`PinEdge`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UartByte {
    /// When.
    pub at: SimTime,
    /// The byte.
    pub byte: u8,
}

/// Why firmware could not be loaded.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum LoadError {
    /// A segment does not fit any ESP32-C3 memory.
    #[error("segment at {addr:#010x} ({len} bytes) is outside the ESP32-C3 memory map")]
    Unmapped {
        /// Segment address.
        addr: u32,
        /// Segment size in memory.
        len: u32,
    },
    /// A flash image built for another chip.
    #[error("image is for chip id {0}, not the ESP32-C3 (5)")]
    WrongChip(u16),
    /// Two flash-window segments need the same flash bytes. IROM and DROM
    /// share one MMU table, so offset `x` of both windows is the same flash
    /// byte under direct boot's identity layout.
    #[error("segment at {addr:#010x} overlaps another flash-window segment in flash")]
    FlashOverlap {
        /// Segment address.
        addr: u32,
    },
}

/// What direct boot loaded, kept so a watchdog reset can boot it again.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BootImage {
    entry: u32,
    chunks: Vec<(u32, Vec<u8>)>,
}

/// How the chip came up, so a reset can do it again.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Boot {
    /// Direct boot of an ELF or image given by the caller.
    Image(BootImage),
    /// ROM-lite boot from the flash contents.
    Flash,
}

/// An ESP32-C3: one RV32IMC hart, its memory map and peripherals.
#[derive(Debug, Clone)]
pub struct Esp32c3 {
    /// The CPU core.
    pub hart: Hart,
    /// Memories and peripherals.
    pub bus: SocBus,
    pins: [PinOut; GPIO_COUNT],
    edges: Vec<PinEdge>,
    uart: Vec<UartByte>,
    boot: Option<Boot>,
    boot_watchdogs: bool,
    resets: Vec<WatchdogReset>,
}

impl Default for Esp32c3 {
    fn default() -> Self {
        Self::new()
    }
}

/// The ESP32-C3 image header's chip id.
pub const CHIP_ID: u16 = 5;

impl Esp32c3 {
    /// Power-on state. The PC is the ROM reset vector until firmware is
    /// loaded. Watchdogs are idle, as if a bootloader had dealt with them;
    /// see [`Self::arm_boot_watchdogs`].
    pub fn new() -> Self {
        let bus = SocBus::new();
        let pins = std::array::from_fn(|p| bus.gpio.pin_out(p));
        Self {
            hart: Hart::new(HartConfig::esp32c3()),
            bus,
            pins,
            edges: Vec::new(),
            uart: Vec::new(),
            boot: None,
            boot_watchdogs: false,
            resets: Vec::new(),
        }
    }

    /// Leave the watchdogs as the ROM does after a power-on reset into
    /// flash boot: TIMG0's MWDT (stage 0 system reset after 325 ms) and the
    /// RWDT (about 1.5 s) under flash boot protection, and the super
    /// watchdog (1 s) running. Firmware must disable or feed them, as on
    /// hardware. They are armed again after every core or system reset.
    pub fn arm_boot_watchdogs(&mut self) {
        self.boot_watchdogs = true;
        self.bus.arm_boot_watchdogs(self.hart.local_time());
    }

    /// Put the mask ROM in place and reset into it, as at power-on: the
    /// hart starts at the reset vector `0x4000_0000` and the ROM boots from
    /// whatever the flash holds (see [`crate::rom`]). Resets later run the
    /// ROM again.
    pub fn power_on_rom(&mut self, rom: &RomImage) {
        // The ROM region is exactly the image size, so this cannot fail.
        let _ = self.bus.load(map::IROM_INTERNAL.0, rom.bytes());
        self.boot = None;
        self.hart.reset();
        if self.bus.strapping == crate::bus::STRAP_SPI_BOOT {
            // Hardware arms the flash boot protection at reset.
            self.arm_boot_watchdogs();
        }
    }

    /// Boot what the flash holds, as the ROM and the second-stage
    /// bootloader would, without executing them ("ROM-lite", see
    /// [`crate::boot`]). Resets boot the flash again.
    pub fn boot_flash(&mut self) -> Result<FlashBoot, FlashBootError> {
        let booted = boot::boot(&mut self.bus)?;
        self.hart.set_pc(booted.entry());
        self.boot = Some(Boot::Flash);
        Ok(booted)
    }

    /// Write `bytes` to the flash chip at `offset` (as `esptool
    /// write_flash` would; erased bytes are `0xFF`).
    pub fn write_flash(&mut self, offset: u32, bytes: &[u8]) {
        self.bus.flash.write_raw(offset, bytes);
    }

    /// Direct boot from an ELF: copy every segment into its memory (`.bss`
    /// zero-filled) and jump to the entry point, as if a bootloader had
    /// done it. Segments in the IROM/DROM cache windows are written to flash
    /// at their window offset and the MMU maps those pages one to one
    /// (enabling the cache), so code and data in both windows must not
    /// share flash bytes (the two windows share one MMU table).
    pub fn load_elf(&mut self, img: &ElfImage) -> Result<(), LoadError> {
        let chunks = img
            .segments
            .iter()
            .map(|s| {
                let mut bytes = s.data.clone();
                bytes.resize(s.mem_size as usize, 0);
                (s.vaddr, bytes)
            })
            .collect();
        self.boot_from(BootImage {
            entry: img.entry,
            chunks,
        })
    }

    /// Direct boot from an ESP flash image (an app `.bin` as written by
    /// `esptool elf2image`): load each segment at its address and jump to
    /// the entry point. Padding segments (load address 0) are skipped.
    /// Flash-window segments are placed as in [`Self::load_elf`] (identity
    /// layout), not where the image file puts them; to boot an image the
    /// way the bootloader does, from its place in flash, use
    /// [`Self::boot_flash`].
    pub fn load_flash_image(&mut self, img: &EspImage) -> Result<(), LoadError> {
        if img.chip_id != CHIP_ID {
            return Err(LoadError::WrongChip(img.chip_id));
        }
        let chunks = img
            .segments
            .iter()
            .filter(|s| s.addr != 0)
            .map(|s| (s.addr, s.data.clone()))
            .collect();
        self.boot_from(BootImage {
            entry: img.entry,
            chunks,
        })
    }

    fn boot_from(&mut self, boot: BootImage) -> Result<(), LoadError> {
        let mut used: Vec<(u32, u32)> = Vec::new();
        for (addr, bytes) in &boot.chunks {
            let unmapped = |BusError { .. }| LoadError::Unmapped {
                addr: *addr,
                len: bytes.len() as u32,
            };
            if within(map::IROM_FLASH, *addr, bytes.len() as u32) || within(map::DROM_FLASH, *addr, bytes.len() as u32)
            {
                let (start, end) = self.bus.place_in_flash(*addr, bytes).map_err(unmapped)?;
                if used.iter().any(|&(s, e)| start < e && s < end) {
                    return Err(LoadError::FlashOverlap { addr: *addr });
                }
                used.push((start, end));
            } else {
                self.bus.load(*addr, bytes).map_err(unmapped)?;
            }
        }
        if !used.is_empty() {
            self.bus.cache.enable();
        }
        self.hart.set_pc(boot.entry);
        self.boot = Some(Boot::Image(boot));
        Ok(())
    }

    /// Run one stretch: until `deadline`, a sync point or a stop. Timer and
    /// watchdog events before the deadline are handled on the way (the hart
    /// stops at each one, so interrupts are raised at the right time); sync
    /// points are handled (writes applied, pin edges and UART bytes stamped)
    /// before returning. [`RunExit::WaitingForInterrupt`] means the hart
    /// sleeps in `wfi` with nothing due before `deadline`.
    pub fn run_slice(&mut self, deadline: SimTime) -> RunExit {
        loop {
            let now = self.hart.local_time();
            let stop = self.bus.next_event().map_or(deadline, |t| t.max(now).min(deadline));
            let exit = self.hart.run_until(&mut self.bus, stop);
            match exit {
                RunExit::SyncRequested(_) => {
                    self.sync();
                    return exit;
                }
                RunExit::BudgetExhausted | RunExit::WaitingForInterrupt if stop < deadline => {
                    self.hart.idle_until(stop);
                    self.sync();
                }
                RunExit::BudgetExhausted => {
                    self.sync();
                    return exit;
                }
                other => return other,
            }
        }
    }

    /// Run until `deadline` or a stop other than a sync point. A hart
    /// sleeping in `wfi` sleeps through to the deadline (returning
    /// [`RunExit::WaitingForInterrupt`]).
    pub fn run_until(&mut self, deadline: SimTime) -> RunExit {
        loop {
            match self.run_slice(deadline) {
                RunExit::SyncRequested(_) => {}
                RunExit::WaitingForInterrupt => {
                    self.hart.idle_until(deadline);
                    return RunExit::WaitingForInterrupt;
                }
                other => return other,
            }
        }
    }

    /// Bring the peripherals to the hart's time: apply pending writes, fire
    /// timers and watchdogs, update the interrupt line, stamp outputs.
    fn sync(&mut self) {
        let now = self.hart.local_time();
        if let Some(r) = self.bus.settle(now) {
            self.watchdog_reset(r);
        }
        self.hart.set_interrupt_lines(self.bus.cpu_lines());
        for byte in self.bus.take_uart_tx() {
            self.uart.push(UartByte { at: now, byte });
        }
        if self.bus.take_pins_dirty() {
            for (pin, old) in self.pins.iter_mut().enumerate() {
                let new = self.bus.gpio.pin_out(pin);
                if new != *old {
                    *old = new;
                    self.edges.push(PinEdge {
                        at: now,
                        pin: pin as u8,
                        out: new,
                    });
                }
            }
        }
    }

    /// Perform a watchdog reset and boot again (the ROM would reload the
    /// app from flash; direct boot reloads the image it was given).
    fn watchdog_reset(&mut self, r: WatchdogReset) {
        self.resets.push(r);
        self.hart.reset();
        self.bus.reset_peripherals(r.kind);
        self.bus.rtc_cntl.set_reset_cause(r.cause_code());
        if self.boot_watchdogs && r.kind != ResetKind::Cpu {
            self.bus.arm_boot_watchdogs(self.hart.local_time());
        }
        match self.boot.take() {
            // It loaded before, so it loads again.
            Some(Boot::Image(image)) => {
                let _ = self.boot_from(image);
            }
            Some(Boot::Flash) => {
                let _ = self.boot_flash();
            }
            // The ROM runs again from the reset vector.
            None => {}
        }
    }

    /// Watchdog resets so far, oldest first.
    pub fn resets(&self) -> &[WatchdogReset] {
        &self.resets
    }

    /// Whether the hart is sleeping in `wfi`.
    pub fn is_sleeping(&self) -> bool {
        self.hart.state() == HartState::WaitingForInterrupt
    }

    /// Current output state of `pin`.
    pub fn pin(&self, pin: usize) -> PinOut {
        self.pins[pin]
    }

    /// All pin edges so far.
    pub fn pin_edges(&self) -> &[PinEdge] {
        &self.edges
    }

    /// Remove and return the pin edges recorded so far.
    pub fn take_pin_edges(&mut self) -> Vec<PinEdge> {
        std::mem::take(&mut self.edges)
    }

    /// Everything written to UART0 so far.
    pub fn uart_bytes(&self) -> &[UartByte] {
        &self.uart
    }

    /// UART0 output as text (lossy UTF-8).
    pub fn uart_text(&self) -> String {
        String::from_utf8_lossy(&self.uart.iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned()
    }
}
