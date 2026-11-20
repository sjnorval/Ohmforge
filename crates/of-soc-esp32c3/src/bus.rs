//! The ESP32-C3 system bus: memories and peripheral dispatch.

use std::collections::BTreeMap;

use of_core::SimTime;
use of_cpu_riscv::{AccessKind, Bus, BusError, Size};

use crate::cache::{self, Cache, CacheBus, CacheFault};
use crate::csr::C3Csrs;
use crate::efuse::Efuse;
use crate::flash::SpiFlash;
use crate::gpio::Gpio;
use crate::intmatrix::{IntMatrix, source};
use crate::ledc::{Ledc, SignalChange};
use crate::map::{self, within};
use crate::rng::{self, Rng};
use crate::rtc_cntl::{RtcCntl, RtcWatchdog};
use crate::saradc::SarAdc;
use crate::sha::Sha;
use crate::spi_mem::SpiMem;
use crate::system::System;
use crate::systimer::Systimer;
use crate::timg::Timg;
use crate::uart::{self, Uart};
use crate::usb_serial::UsbSerialJtag;
use crate::watchdog::{ResetKind, ResetSource, WatchdogReset};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Mem {
    RomI,
    RomD,
    Sram,
    Rtc,
}

/// Memories plus peripherals, as seen from the hart.
#[derive(Debug, Clone)]
pub struct SocBus {
    rom: Vec<u8>,
    sram: Vec<u8>,
    rtc: Vec<u8>,
    /// The external SPI flash chip, seen through the cache windows.
    pub flash: SpiFlash,
    /// Cache controller and MMU (maps flash into the IROM/DROM windows).
    pub cache: Cache,
    /// SPI memory controllers: `spi[0]` is SPI0 (cache side), `spi[1]` is
    /// SPI1 (software flash commands).
    pub spi: [SpiMem; 2],
    /// C3-specific CSRs (performance counters, user trap CSRs).
    pub csrs: C3Csrs,
    /// Strapping pins latched at reset, as `GPIO_STRAP_REG` reports them
    /// ([`STRAP_SPI_BOOT`] by default).
    pub strapping: u32,
    /// USB Serial/JTAG controller (console output only).
    pub usb_serial: UsbSerialJtag,
    /// SHA accelerator.
    pub sha: Sha,
    /// eFuse controller (all fuses blank).
    pub efuse: Efuse,
    /// Random number generator (deterministic).
    pub rng: Rng,
    /// GPIO, GPIO matrix and IO MUX.
    pub gpio: Gpio,
    /// SYSTEM clock configuration (the CPU clock).
    pub system: System,
    /// SYSTIMER.
    pub systimer: Systimer,
    /// Timer groups 0 and 1 (timer and MWDT each).
    pub timg: [Timg; 2],
    /// RTC_CNTL watchdogs (RWDT, SWD).
    pub rtc_cntl: RtcCntl,
    /// Interrupt matrix and CPU interrupt controller.
    pub intmatrix: IntMatrix,
    /// LED PWM controller.
    pub ledc: Ledc,
    /// SAR ADC digital controller (one-time sampling).
    pub saradc: SarAdc,
    /// UART0 configuration (the bytes it sends are in `uart_tx`).
    pub uart0: Uart,
    uart_tx: Vec<u8>,
    /// Writes to timed peripherals, applied at the next [`Self::settle`].
    deferred: Vec<(u32, u32)>,
    regs: BTreeMap<u32, u32>,
    unmodelled: BTreeMap<u32, u64>,
    sync: bool,
    pins_dirty: bool,
    input_horizon: Option<SimTime>,
}

impl Default for SocBus {
    fn default() -> Self {
        Self::new()
    }
}

/// `GPIO_STRAP_REG` for SPI flash boot (GPIO9 and GPIO8 high): the ROM
/// reports `boot:0xc (SPI_FAST_FLASH_BOOT)`.
pub const STRAP_SPI_BOOT: u32 = 0x0c;
/// `GPIO_STRAP_REG` for download mode (GPIO9 low, GPIO8 high).
pub const STRAP_DOWNLOAD: u32 = 0x04;
const GPIO_STRAP: u32 = 0x38;

impl SocBus {
    /// Power-on state: zeroed memories, an erased 4 MiB flash, peripherals
    /// at reset (cache disabled, MMU entries invalid).
    pub fn new() -> Self {
        Self::with_flash(SpiFlash::default())
    }

    /// Power-on state with the given flash chip.
    pub fn with_flash(flash: SpiFlash) -> Self {
        Self {
            rom: vec![0; map::IROM_INTERNAL.1 as usize],
            sram: vec![0; map::IRAM.1 as usize],
            rtc: vec![0; map::RTC_FAST.1 as usize],
            flash,
            cache: Cache::new(),
            spi: [SpiMem::new(), SpiMem::new()],
            csrs: C3Csrs::new(),
            strapping: STRAP_SPI_BOOT,
            usb_serial: UsbSerialJtag::new(),
            sha: Sha::new(),
            efuse: Efuse::new(),
            rng: Rng::new(),
            gpio: Gpio::new(),
            system: System::new(),
            systimer: Systimer::new(),
            timg: [Timg::new(), Timg::new()],
            rtc_cntl: RtcCntl::new(),
            intmatrix: IntMatrix::new(),
            ledc: Ledc::new(),
            saradc: SarAdc::new(),
            uart0: Uart::new(),
            uart_tx: Vec::new(),
            deferred: Vec::new(),
            regs: BTreeMap::new(),
            unmodelled: BTreeMap::new(),
            sync: false,
            pins_dirty: false,
            input_horizon: None,
        }
    }

    fn locate(&self, addr: u32, len: u32) -> Option<(Mem, usize)> {
        let table = [
            (Mem::RomI, map::IROM_INTERNAL),
            // DROM_INTERNAL is ROM1 (the upper 128 KiB of the ROM).
            (Mem::RomD, map::DROM_INTERNAL),
            (Mem::Sram, map::IRAM),
            (Mem::Sram, map::DRAM),
            (Mem::Rtc, map::RTC_FAST),
        ];
        let (m, region) = table.into_iter().find(|&(_, r)| within(r, addr, len))?;
        let mut off = (addr - region.0) as usize;
        if m == Mem::RomD {
            off += 0x4_0000;
        }
        let backing = self.mem(m).len();
        (off + len as usize <= backing).then_some((m, off))
    }

    fn mem(&self, m: Mem) -> &[u8] {
        match m {
            Mem::RomI | Mem::RomD => &self.rom,
            Mem::Sram => &self.sram,
            Mem::Rtc => &self.rtc,
        }
    }

    fn mem_mut(&mut self, m: Mem) -> &mut [u8] {
        match m {
            Mem::RomI | Mem::RomD => &mut self.rom,
            Mem::Sram => &mut self.sram,
            Mem::Rtc => &mut self.rtc,
        }
    }

    /// Which flash window `addr..addr+len` lies in, if any.
    fn window(addr: u32, len: u32) -> Option<CacheBus> {
        if within(map::IROM_FLASH, addr, len) {
            Some(CacheBus::IBus)
        } else if within(map::DROM_FLASH, addr, len) {
            Some(CacheBus::DBus)
        } else {
            None
        }
    }

    /// The flash address behind a window address according to the MMU
    /// entry alone (cache enable and bus shut-off ignored; for loaders and
    /// debuggers).
    pub fn mmu_lookup(&self, addr: u32) -> Option<u32> {
        let linear = addr & (map::IROM_FLASH.1 - 1);
        let e = self.cache.entry((linear / cache::PAGE) as usize);
        (e & cache::MMU_INVALID == 0).then(|| (e & cache::MMU_PAGE_MASK) * cache::PAGE + linear % cache::PAGE)
    }

    /// Direct-boot placement of bytes meant for a cache window: map every
    /// 64 KiB page they touch to the flash page with the same number (an
    /// identity layout, flash offset = window offset), write them to flash
    /// and return the flash range used. Fails if the range is outside the
    /// flash chip.
    pub fn place_in_flash(&mut self, addr: u32, bytes: &[u8]) -> Result<(u32, u32), BusError> {
        let len = bytes.len() as u32;
        if Self::window(addr, len).is_none() {
            return Err(BusError { addr });
        }
        let start = addr & (map::IROM_FLASH.1 - 1);
        let end = start + len;
        if end as usize > self.flash.size() {
            return Err(BusError { addr });
        }
        for n in start / cache::PAGE..end.div_ceil(cache::PAGE) {
            self.cache.map(n as usize, n);
        }
        self.flash.write_raw(start, bytes);
        Ok((start, end))
    }

    /// Back-door write used by loaders: also fills ROM, which the hart
    /// cannot write, and the flash behind a mapped cache window.
    pub fn load(&mut self, addr: u32, bytes: &[u8]) -> Result<(), BusError> {
        if Self::window(addr, bytes.len() as u32).is_some() {
            for (i, b) in bytes.iter().enumerate() {
                let a = addr + i as u32;
                let off = self.mmu_lookup(a).ok_or(BusError { addr: a })?;
                self.flash.write_raw(off, &[*b]);
            }
            return Ok(());
        }
        let (m, off) = self.locate(addr, bytes.len() as u32).ok_or(BusError { addr })?;
        self.mem_mut(m)[off..off + bytes.len()].copy_from_slice(bytes);
        Ok(())
    }

    /// Read memory without side effects (peripherals excluded). Flash
    /// windows are read through the MMU entries (the range must stay
    /// inside one 64 KiB page).
    pub fn peek(&self, addr: u32, len: usize) -> Option<&[u8]> {
        if Self::window(addr, len as u32).is_some() {
            let off = self.mmu_lookup(addr)? as usize;
            let same_page = (addr % cache::PAGE) as usize + len <= cache::PAGE as usize;
            return (same_page && off + len <= self.flash.size()).then(|| &self.flash.bytes()[off..off + len]);
        }
        let (m, off) = self.locate(addr, len as u32)?;
        Some(&self.mem(m)[off..off + len])
    }

    /// A read through a cache window, as the hart sees it.
    fn window_read(&self, addr: u32, size: Size, bus: CacheBus) -> Result<u32, CacheFault> {
        let mut bytes = [0u8; 4];
        let n = size.bytes();
        if addr % cache::PAGE + n <= cache::PAGE {
            // One translation for the whole access.
            let base = self.cache.translate(addr, bus)?;
            for (i, b) in bytes.iter_mut().enumerate().take(n as usize) {
                *b = self.flash.byte(base + i as u32);
            }
        } else {
            for (i, b) in bytes.iter_mut().enumerate().take(n as usize) {
                *b = self.flash.byte(self.cache.translate(addr + i as u32, bus)?);
            }
        }
        Ok(get(&bytes, size))
    }

    /// Bytes written to UART0 since the last [`Self::take_uart_tx`].
    pub fn take_uart_tx(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.uart_tx)
    }

    /// LEDC output changes since the last call, in time order (the pins
    /// they drive are not updated yet; see [`Self::apply_signal`]).
    pub fn take_signal_changes(&mut self) -> Vec<SignalChange> {
        self.ledc.take_changes()
    }

    /// Put an LEDC output change on its GPIO matrix signal.
    pub fn apply_signal(&mut self, c: SignalChange) {
        self.gpio
            .set_output_signal(usize::from(crate::ledc::SIGNAL_BASE + c.channel), c.level);
    }

    /// Whether GPIO output state may have changed since the last call.
    pub fn take_pins_dirty(&mut self) -> bool {
        std::mem::take(&mut self.pins_dirty)
    }

    /// Defer loads of the GPIO input registers (`GPIO_IN`, `GPIO_STATUS`,
    /// `GPIO_PCPU_INT`, `GPIO_PCPU_NMI_INT`) made after `horizon`, the time
    /// up to which outside pin levels are known (see
    /// [`of_cpu_riscv::Bus::defer_load`]). `None` (the default) never
    /// defers: pin levels are whatever was last set.
    pub fn set_input_horizon(&mut self, horizon: Option<SimTime>) {
        self.input_horizon = horizon;
    }

    /// Accesses to peripheral registers that are not modelled, by word
    /// address, with access counts. Useful to see what firmware expects.
    pub fn unmodelled(&self) -> &BTreeMap<u32, u64> {
        &self.unmodelled
    }

    /// Arm the watchdogs the ROM leaves running after a power-on reset into
    /// flash boot (TIMG0 MWDT and RWDT flash boot protection, SWD) at `t`.
    pub fn arm_boot_watchdogs(&mut self, t: SimTime) {
        self.timg[0].arm_boot_protection(t);
        self.rtc_cntl.arm_boot_watchdogs(t);
    }

    /// Reset the peripherals a `kind` reset clears: digital peripherals for
    /// a core or system reset, the RTC watchdogs too for a system reset.
    /// Memories are left alone.
    pub fn reset_peripherals(&mut self, kind: ResetKind) {
        // The core's own CSRs reset with it.
        self.csrs = C3Csrs::new();
        if kind == ResetKind::Cpu {
            return;
        }
        self.sha = Sha::new();
        self.uart0 = Uart::new();
        self.gpio.reset();
        self.cache = Cache::new();
        self.spi = [SpiMem::new(), SpiMem::new()];
        self.system = System::new();
        self.systimer = Systimer::new();
        self.timg = [Timg::new(), Timg::new()];
        self.intmatrix = IntMatrix::new();
        // With the GPIO matrix (its signal levels are cleared too).
        self.ledc = Ledc::new();
        self.saradc.reset();
        self.deferred.clear();
        self.pins_dirty = true;
        if kind == ResetKind::System {
            self.rtc_cntl = RtcCntl::new();
        }
    }

    /// Earliest time at which a timer or watchdog needs [`Self::settle`].
    pub fn next_event(&self) -> Option<SimTime> {
        [
            self.systimer.next_event(),
            self.timg[0].next_event(),
            self.timg[1].next_event(),
            self.rtc_cntl.next_event(),
            self.ledc.next_event(),
            self.saradc.next_event(),
        ]
        .into_iter()
        .flatten()
        .min()
    }

    fn advance(&mut self, now: SimTime) -> Option<WatchdogReset> {
        self.systimer.advance(now);
        self.ledc.advance(now);
        self.saradc.advance(now);
        let mut resets = Vec::new();
        for (g, timg) in (0u8..).zip(&mut self.timg) {
            if let Some((at, kind)) = timg.advance(now) {
                resets.push(WatchdogReset {
                    at,
                    kind,
                    source: ResetSource::Mwdt(g),
                });
            }
        }
        if let Some((at, kind, which)) = self.rtc_cntl.advance(now) {
            let source = match which {
                RtcWatchdog::Rwdt => ResetSource::Rwdt,
                RtcWatchdog::Swd => ResetSource::Swd,
            };
            resets.push(WatchdogReset { at, kind, source });
        }
        resets.into_iter().min_by_key(|r| (r.at, std::cmp::Reverse(r.kind)))
    }

    /// Bring timers and watchdogs up to `now`, apply the writes made by
    /// the last instruction (at `now`) and update the interrupt sources.
    /// Returns a watchdog reset if one is due; the caller performs it.
    pub fn settle(&mut self, now: SimTime) -> Option<WatchdogReset> {
        let mut reset = self.advance(now);
        if reset.is_none() {
            for (addr, v) in std::mem::take(&mut self.deferred) {
                let block = addr & !(map::BLOCK - 1);
                let off = addr - block;
                match block {
                    map::SYSTIMER_BASE => self.systimer.write(off, v, now),
                    map::TIMG0_BASE => self.timg[0].write(off, v, now),
                    map::TIMG1_BASE => self.timg[1].write(off, v, now),
                    map::RTC_CNTL_BASE => self.rtc_cntl.write(off, v, now),
                    map::LEDC_BASE => self.ledc.write(off, v, now),
                    map::APB_SARADC_BASE => self.saradc.write(off, v, now),
                    _ => None,
                };
            }
            reset = self.advance(now);
        }
        self.deferred.clear();
        self.update_sources();
        reset
    }

    fn update_sources(&mut self) {
        let st = |bit: u32, src: u8| u64::from(bit != 0) << src;
        let (g0, g1) = (self.timg[0].int_status(), self.timg[1].int_status());
        let (gpio_cpu, gpio_nmi) = self.gpio.irq();
        let levels = (u64::from(self.systimer.int_status()) << source::SYSTIMER_TARGET0)
            | st(g0 & 1, source::TG0_T0)
            | st(g0 & 2, source::TG0_WDT)
            | st(g1 & 1, source::TG1_T0)
            | st(g1 & 2, source::TG1_WDT)
            | st(u32::from(self.rtc_cntl.irq()), source::RTC_CORE)
            | st(u32::from(self.ledc.irq()), source::LEDC)
            | st(u32::from(self.saradc.irq()), source::APB_ADC)
            | st(u32::from(gpio_cpu), source::GPIO)
            | st(u32::from(gpio_nmi), source::GPIO_NMI);
        self.intmatrix.set_sources(levels);
    }

    /// Update the interrupt sources after a pin input changed outside an
    /// instruction (see [`Gpio::set_input`]).
    pub fn inputs_changed(&mut self) {
        self.update_sources();
    }

    /// Interrupt lines for the hart: the controller's winning interrupt.
    pub fn cpu_lines(&self) -> u32 {
        self.intmatrix.cpu_lines()
    }

    /// Whether a write to `block + off` must wait for the hart's time.
    fn timed_reg(block: u32, off: u32) -> bool {
        match block {
            map::SYSTIMER_BASE => Systimer::is_reg(off),
            map::TIMG0_BASE | map::TIMG1_BASE => Timg::is_reg(off),
            map::RTC_CNTL_BASE => RtcCntl::is_reg(off),
            map::LEDC_BASE => Ledc::is_reg(off),
            map::APB_SARADC_BASE => SarAdc::is_reg(off),
            _ => false,
        }
    }

    fn periph_read(&mut self, addr: u32) -> u32 {
        let block = addr & !(map::BLOCK - 1);
        let off = addr - block;
        let v = match block {
            map::GPIO_BASE if off == GPIO_STRAP => Some(self.strapping),
            map::GPIO_BASE => self.gpio.read(off),
            map::RTC_CNTL_BASE if off >= map::EFUSE_OFFSET => self.efuse.read(off - map::EFUSE_OFFSET),
            map::RTC_CNTL_BASE if self.rtc_cntl.read_static(off).is_some() => self.rtc_cntl.read_static(off),
            map::USB_SERIAL_JTAG_BASE => self.usb_serial.read(off),
            map::APB_CTRL_BASE if off == rng::DATA => Some(self.rng.next_word()),
            map::SHA_BASE => self.sha.read(off),
            map::IO_MUX_BASE => self.gpio.io_mux_read(off),
            map::SYSTIMER_BASE => self.systimer.read(off),
            map::TIMG0_BASE => self.timg[0].read(off),
            map::TIMG1_BASE => self.timg[1].read(off),
            map::RTC_CNTL_BASE => self.rtc_cntl.read(off),
            map::LEDC_BASE => self.ledc.read(off),
            map::APB_SARADC_BASE => self.saradc.read(off),
            map::INTERRUPT_BASE => self.intmatrix.read(off),
            map::SYSTEM_BASE => self.system.read(off),
            map::SPI0_BASE => self.spi[0].read(off),
            map::SPI1_BASE => self.spi[1].read(off),
            map::EXTMEM_BASE => self.cache.read(off),
            map::MMU_TABLE_BASE => self.cache.mmu_read(off),
            map::UART0_BASE if off == uart::reg::FIFO => Some(u32::from(self.uart0.pop_rx())),
            map::UART0_BASE => self.uart0.read(off),
            _ => None,
        };
        v.unwrap_or_else(|| {
            *self.unmodelled.entry(addr).or_default() += 1;
            self.regs.get(&addr).copied().unwrap_or(0)
        })
    }

    fn periph_write(&mut self, addr: u32, v: u32) {
        let block = addr & !(map::BLOCK - 1);
        let off = addr - block;
        let handled = match block {
            map::GPIO_BASE if off == GPIO_STRAP => Some(()),
            map::GPIO_BASE => self.gpio.write(off, v).map(|()| self.pins_dirty = true),
            map::RTC_CNTL_BASE if off >= map::EFUSE_OFFSET => self.efuse.write(off - map::EFUSE_OFFSET, v),
            map::USB_SERIAL_JTAG_BASE => self.usb_serial.write(off, v),
            map::SHA_BASE => self.sha.write(off, v),
            map::IO_MUX_BASE => self.gpio.io_mux_write(off, v).map(|()| self.pins_dirty = true),
            map::SYSTIMER_BASE
            | map::TIMG0_BASE
            | map::TIMG1_BASE
            | map::RTC_CNTL_BASE
            | map::LEDC_BASE
            | map::APB_SARADC_BASE
                if Self::timed_reg(block, off) =>
            {
                self.deferred.push((addr, v));
                Some(())
            }
            map::RTC_CNTL_BASE if self.rtc_cntl.write_static(off, v).is_some() => Some(()),
            map::INTERRUPT_BASE => self.intmatrix.write(off, v),
            map::SYSTEM_BASE => self.system.write(off, v),
            map::SPI0_BASE => self.spi[0].write(off, v, &mut self.flash),
            map::SPI1_BASE => self.spi[1].write(off, v, &mut self.flash),
            map::EXTMEM_BASE => self.cache.write(off, v),
            map::MMU_TABLE_BASE => self.cache.mmu_write(off, v),
            map::UART0_BASE if off == uart::reg::FIFO => {
                self.uart_tx.push(v as u8);
                Some(())
            }
            map::UART0_BASE => self.uart0.write(off, v),
            _ => None,
        };
        if handled.is_none() {
            *self.unmodelled.entry(addr).or_default() += 1;
            self.regs.insert(addr, v);
        }
    }
}

fn get(bytes: &[u8], size: Size) -> u32 {
    match size {
        Size::Byte => u32::from(bytes[0]),
        Size::Half => u32::from(u16::from_le_bytes([bytes[0], bytes[1]])),
        Size::Word => u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
    }
}

impl Bus for SocBus {
    fn read(&mut self, addr: u32, size: Size, kind: AccessKind) -> Result<u32, BusError> {
        let n = size.bytes();
        if within(map::PERIPHERALS, addr, n) {
            if kind == AccessKind::Fetch {
                return Err(BusError { addr });
            }
            self.sync = true;
            let word = self.periph_read(addr & !3);
            let shift = 8 * (addr & 3);
            return Ok(match size {
                Size::Byte => (word >> shift) & 0xff,
                Size::Half => (word >> shift) & 0xffff,
                Size::Word => word,
            });
        }
        if let Some(bus) = Self::window(addr, n) {
            if kind == AccessKind::Fetch && bus == CacheBus::DBus {
                return Err(BusError { addr });
            }
            return self.window_read(addr, size, bus).map_err(|_| BusError { addr });
        }
        let (m, off) = self.locate(addr, n).ok_or(BusError { addr })?;
        if kind == AccessKind::Fetch && m == Mem::RomD {
            return Err(BusError { addr });
        }
        if kind == AccessKind::Fetch && m == Mem::Sram && within(map::DRAM, addr, n) {
            return Err(BusError { addr });
        }
        Ok(get(&self.mem(m)[off..], size))
    }

    fn write(&mut self, addr: u32, size: Size, value: u32, _kind: AccessKind) -> Result<(), BusError> {
        let n = size.bytes();
        if within(map::PERIPHERALS, addr, n) {
            self.sync = true;
            let shift = 8 * (addr & 3);
            self.periph_write(addr & !3, value << shift);
            return Ok(());
        }
        let (m, off) = self.locate(addr, n).ok_or(BusError { addr })?;
        if !matches!(m, Mem::Sram | Mem::Rtc) {
            return Err(BusError { addr });
        }
        self.mem_mut(m)[off..off + n as usize].copy_from_slice(&value.to_le_bytes()[..n as usize]);
        Ok(())
    }

    fn defer_load(&mut self, addr: u32, now: SimTime) -> bool {
        use crate::gpio::reg;
        self.input_horizon.is_some_and(|h| now > h)
            && (addr & !3)
                .checked_sub(map::GPIO_BASE)
                .is_some_and(|off| matches!(off, reg::IN | reg::STATUS | reg::PCPU_INT | reg::PCPU_NMI_INT))
    }

    fn take_sync_request(&mut self) -> bool {
        std::mem::take(&mut self.sync)
    }

    fn csr_cycles(&mut self, cycles: u64) {
        self.csrs.set_cycles(cycles);
    }

    fn csr_read(&mut self, csr: u16) -> Option<u32> {
        self.csrs.read(csr)
    }

    fn csr_write(&mut self, csr: u16, value: u32) -> bool {
        self.csrs.write(csr, value)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn iram_and_dram_alias_the_same_sram() {
        let mut b = SocBus::new();
        b.write(0x3fc8_0000, Size::Word, 0xdead_beef, AccessKind::Store)
            .unwrap();
        assert_eq!(b.read(0x4038_0000, Size::Word, AccessKind::Fetch), Ok(0xdead_beef));
        assert_eq!(b.read(0x4038_0002, Size::Half, AccessKind::Load), Ok(0xdead));
        assert_eq!(b.read(0x3fc8_0000, Size::Byte, AccessKind::Load), Ok(0xef));
        assert!(
            b.read(0x3fc8_0000, Size::Word, AccessKind::Fetch).is_err(),
            "DRAM is not executable"
        );
        // Top of SRAM.
        b.write(0x3fcd_fffc, Size::Word, 7, AccessKind::Store).unwrap();
        assert!(b.write(0x3fce_0000, Size::Word, 7, AccessKind::Store).is_err());
    }

    #[test]
    fn flash_and_rom_are_read_only_to_the_hart() {
        let mut b = SocBus::new();
        assert!(b.load(0x4200_0020, &[0x13, 0, 0, 0]).is_err(), "page not mapped yet");
        b.place_in_flash(0x4200_0020, &[0x13, 0, 0, 0]).unwrap();
        b.place_in_flash(0x3c01_0020, &[1, 2, 3, 4]).unwrap();
        b.cache.enable();
        assert_eq!(b.read(0x4200_0020, Size::Word, AccessKind::Fetch), Ok(0x13));
        assert_eq!(b.read(0x3c01_0020, Size::Word, AccessKind::Load), Ok(0x0403_0201));
        b.load(0x3c01_0020, &[5]).unwrap();
        assert_eq!(b.flash.byte(0x1_0020), 5, "loads go through the MMU to flash");
        assert!(b.write(0x4200_0020, Size::Word, 0, AccessKind::Store).is_err());
        assert!(b.write(0x4000_0000, Size::Word, 0, AccessKind::Store).is_err());
        assert!(b.read(0x3c01_0020, Size::Word, AccessKind::Fetch).is_err());
        // ROM1 through the data bus.
        b.load(0x4004_0000, &[9, 0, 0, 0]).unwrap();
        assert_eq!(b.read(0x3ff0_0000, Size::Word, AccessKind::Load), Ok(9));
        assert!(b.read(0x2000_0000, Size::Word, AccessKind::Load).is_err(), "unmapped");
    }

    #[test]
    fn flash_windows_read_through_the_mmu_registers() {
        let mut b = SocBus::new();
        b.flash.write_raw(0x3_0000, &[0xaa, 0xbb, 0xcc, 0xdd]);
        b.flash.write_raw(0x5_fffe, &[0x11, 0x22]);
        b.flash.write_raw(0x7_0000, &[0x33, 0x44]);
        assert!(
            b.read(0x4200_0000, Size::Word, AccessKind::Fetch).is_err(),
            "cache off at reset"
        );
        // What a bootloader does: map pages, enable the cache, open both buses.
        let st = |b: &mut SocBus, a: u32, v: u32| b.write(a, Size::Word, v, AccessKind::Store).unwrap();
        st(&mut b, 0x600c_5000, 3); // entry 0 -> flash page 3
        st(&mut b, 0x600c_5000 + 4 * 0x22, 5); // entry 0x22 -> page 5
        st(&mut b, 0x600c_5000 + 4 * 0x23, 7); // entry 0x23 -> page 7
        st(&mut b, 0x600c_4000, 1);
        st(&mut b, 0x600c_4004, 0);
        assert_eq!(b.read(0x4200_0000, Size::Word, AccessKind::Fetch), Ok(0xddcc_bbaa));
        assert_eq!(
            b.read(0x3c00_0002, Size::Half, AccessKind::Load),
            Ok(0xddcc),
            "same entry on DBus"
        );
        // A fetch straddling two pages is translated byte by byte.
        assert_eq!(b.read(0x4222_fffe, Size::Word, AccessKind::Fetch), Ok(0x4433_2211));
        assert_eq!(b.read(0x600c_5000 + 4 * 0x22, Size::Word, AccessKind::Load), Ok(5));
        assert_eq!(b.peek(0x3c00_0000, 2), Some(&[0xaa, 0xbb][..]));
        // Invalidating the entry makes the page inaccessible.
        st(&mut b, 0x600c_5000, 0x100);
        assert!(b.read(0x3c00_0000, Size::Word, AccessKind::Load).is_err());
        // Shut DBus only.
        st(&mut b, 0x600c_4004, 0b10);
        assert!(b.read(0x3c22_0000, Size::Word, AccessKind::Load).is_err());
        assert!(b.read(0x4222_0000, Size::Word, AccessKind::Fetch).is_ok());
        assert!(b.unmodelled().is_empty());
    }

    #[test]
    fn spi1_reads_the_flash_chip() {
        let mut b = SocBus::new();
        b.flash.write_raw(0x8000, &[0xaa, 0x50, 0x00, 0x00]);
        let st = |b: &mut SocBus, a: u32, v: u32| b.write(a, Size::Word, v, AccessKind::Store).unwrap();
        st(&mut b, 0x6000_2004, 0x8000);
        st(&mut b, 0x6000_2028, 31);
        st(&mut b, 0x6000_2000, 1 << 31);
        assert_eq!(b.read(0x6000_2000, Size::Word, AccessKind::Load), Ok(0));
        assert_eq!(b.read(0x6000_2058, Size::Word, AccessKind::Load), Ok(0x50aa));
        st(&mut b, 0x6000_2000, 1 << 28);
        assert_eq!(
            b.read(0x6000_2058, Size::Word, AccessKind::Load),
            Ok(0x16_40c8),
            "JEDEC ID"
        );
        assert!(b.unmodelled().is_empty());
    }

    #[test]
    fn peripheral_access_requests_sync_and_reaches_gpio() {
        let mut b = SocBus::new();
        assert!(!b.take_sync_request());
        b.write(0x6000_4024, Size::Word, 1 << 2, AccessKind::Store).unwrap();
        b.write(0x6000_4008, Size::Word, 1 << 2, AccessKind::Store).unwrap();
        assert!(b.take_sync_request());
        assert!(!b.take_sync_request(), "taken once");
        assert!(b.take_pins_dirty());
        assert_eq!(b.gpio.pin_out(2).level, Some(true));
        assert_eq!(b.read(0x6000_4004, Size::Word, AccessKind::Load), Ok(1 << 2));
        assert!(b.read(0x6000_4004, Size::Word, AccessKind::Fetch).is_err());
    }

    #[test]
    fn uart_fifo_and_unmodelled_registers() {
        let mut b = SocBus::new();
        for c in b"hi" {
            b.write(0x6000_0000, Size::Word, u32::from(*c), AccessKind::Store)
                .unwrap();
        }
        assert_eq!(b.take_uart_tx(), b"hi");
        // RTC_CNTL_OPTIONS0: not modelled, reads back.
        b.write(0x6000_8000, Size::Word, 0x1c00_a000, AccessKind::Store)
            .unwrap();
        assert_eq!(b.read(0x6000_8000, Size::Word, AccessKind::Load), Ok(0x1c00_a000));
        assert_eq!(b.unmodelled().get(&0x6000_8000), Some(&2));
    }

    #[test]
    fn uart_rx_fifo_readable_from_bus() {
        let mut b = SocBus::new();
        b.uart0.push_rx(b"x");
        let status = b.read(0x6000_001c, Size::Word, AccessKind::Load).unwrap();
        assert_eq!(status & 0x3ff, 1);
        assert_eq!(
            b.read(0x6000_0000, Size::Word, AccessKind::Load).unwrap() & 0xff,
            u32::from(b'x')
        );
        assert_eq!(b.read(0x6000_001c, Size::Word, AccessKind::Load).unwrap() & 0x3ff, 0);
    }

    #[test]
    fn timed_writes_apply_at_settle_and_raise_interrupts() {
        let mut b = SocBus::new();
        let t = SimTime::from_us(10);
        // Route SYSTIMER_TARGET0 to CPU interrupt 3, priority 2.
        b.write(0x600c_2000 + 4 * 37, Size::Word, 3, AccessKind::Store).unwrap();
        b.write(0x600c_2114 + 4 * 3, Size::Word, 2, AccessKind::Store).unwrap();
        b.write(0x600c_2104, Size::Word, 1 << 3, AccessKind::Store).unwrap();
        // Comparator 0: target 16 ticks after 10 µs (11 µs).
        b.write(0x6002_3020, Size::Word, 176, AccessKind::Store).unwrap();
        assert_eq!(b.systimer.read(0x20), Some(0), "deferred until settle");
        assert_eq!(b.settle(t), None);
        assert_eq!(b.systimer.read(0x20), Some(176));
        for (off, v) in [(0x50, 1), (0x64, 1), (0x00, 0x4600_0000 | 1 << 24)] {
            b.write(0x6002_3000 + off, Size::Word, v, AccessKind::Store).unwrap();
            b.settle(t);
        }
        assert_eq!(b.next_event(), Some(SimTime::from_us(11)));
        assert_eq!(b.cpu_lines(), 0);
        b.settle(SimTime::from_us(11));
        assert_eq!(b.cpu_lines(), 1 << 3);
        assert_eq!(b.read(0x600c_20fc, Size::Word, AccessKind::Load), Ok(1 << (37 - 32)));
        // ISR clears the interrupt.
        b.write(0x6002_306c, Size::Word, 1, AccessKind::Store).unwrap();
        b.settle(SimTime::from_us(12));
        assert_eq!(b.cpu_lines(), 0);
        assert!(b.unmodelled().is_empty());
    }

    #[test]
    fn watchdog_resets_are_reported_and_clear_peripherals() {
        let mut b = SocBus::new();
        b.arm_boot_watchdogs(SimTime::ZERO);
        assert_eq!(b.next_event(), Some(SimTime::from_ms(325)));
        let r = b.settle(SimTime::from_ms(400)).unwrap();
        assert_eq!(r.source, ResetSource::Mwdt(0));
        assert_eq!(r.kind, ResetKind::System);
        assert_eq!(r.at, SimTime::from_ms(325));
        b.write(0x6000_4024, Size::Word, 1 << 2, AccessKind::Store).unwrap();
        b.reset_peripherals(ResetKind::Core);
        assert_eq!(b.gpio.read(0x20), Some(0), "GPIO_ENABLE back to reset");
        assert_eq!(b.next_event(), Some(SimTime::from_ms(1000)), "core reset keeps the SWD");
        b.reset_peripherals(ResetKind::System);
        assert_eq!(b.next_event(), None);
    }
}
