//! UART0 (TRM chapter 26): transmit capture, receive FIFO, configuration
//! registers, baud-timed RX and receive interrupts.
//!
//! Bytes written to `UART_FIFO` are captured instantly: the TX FIFO never
//! fills, so `UART_STATUS.TXFIFO_CNT` reads 0, `INT_RAW.TXFIFO_EMPTY` stays
//! set and `UART_FSM_STATUS` reads idle.
//!
//! Host (or test) bytes queued with [`Uart::queue_rx`] arrive in the 128-byte
//! RX FIFO one frame at a time at the configured baud rate (start + data +
//! optional parity + stop). [`Uart::push_rx`] still deposits bytes instantly,
//! for FIFO unit tests. Overfill (a byte arriving into a full FIFO) drops the
//! new byte and sets `INT_RAW.RXFIFO_OVF`, unless `CONF1.DIS_RX_DAT_OVF` is
//! set. `STATUS.RXFIFO_CNT` is the depth.
//!
//! Receive interrupts follow the TRM raw/status/enable/clear registers:
//! `RXFIFO_FULL` (count > `CONF1.RXFIFO_FULL_THRHD`), `RXFIFO_TOUT` (FIFO
//! holds data, `CONF1.RX_TOUT_EN` is set, and no byte arrives for
//! `MEM_CONF.RX_TOUT_THRHD` bit times), and `RXFIFO_OVF`. `INT_ST` is
//! `INT_RAW & INT_ENA`. `INT_CLR` is write-1-to-clear; level conditions
//! (`RXFIFO_FULL`, `TXFIFO_EMPTY`) set themselves again while they still
//! hold.
//!
//! The configuration registers (`INT_ENA`, `CLKDIV`, `CONF0`, `CONF1`,
//! `SLEEP_CONF`, `IDLE_CONF`, `MEM_CONF`, `CLK_CONF`, `ID`) hold what is
//! written, with the TRM reset values, and [`Uart::baud`] works out the
//! configured baud rate from `CLK_CONF` and `CLKDIV`.

use std::collections::VecDeque;

use of_core::SimTime;

/// Register offsets from [`crate::map::UART0_BASE`].
pub mod reg {
    /// TX/RX FIFO.
    pub const FIFO: u32 = 0x00;
    /// Raw interrupt status.
    pub const INT_RAW: u32 = 0x04;
    /// Masked interrupt status.
    pub const INT_ST: u32 = 0x08;
    /// Interrupt enable.
    pub const INT_ENA: u32 = 0x0c;
    /// Interrupt clear (write-1-to-clear).
    pub const INT_CLR: u32 = 0x10;
    /// Baud divider: integer part bits 11:0, `FRAG` (sixteenths) bits 23:20.
    pub const CLKDIV: u32 = 0x14;
    /// Status: `RXFIFO_CNT` bits 9:0, `TXFIFO_CNT` bits 25:16.
    pub const STATUS: u32 = 0x1c;
    /// Frame format and control.
    pub const CONF0: u32 = 0x20;
    /// FIFO thresholds and `RX_TOUT_EN` (bit 21).
    pub const CONF1: u32 = 0x24;
    /// Light-sleep wake-up threshold.
    pub const SLEEP_CONF: u32 = 0x38;
    /// Idle and TX break timing.
    pub const IDLE_CONF: u32 = 0x48;
    /// FIFO memory allocation and `RX_TOUT_THRHD` (bits 25:16).
    pub const MEM_CONF: u32 = 0x60;
    /// Transmit and receive state machines (read-only).
    pub const FSM_STATUS: u32 = 0x6c;
    /// Core clock: `SCLK_DIV_B` 5:0, `SCLK_DIV_A` 11:6, `SCLK_DIV_NUM`
    /// 19:12, `SCLK_SEL` 21:20 (1 APB, 2 RC_FAST, 3 XTAL), enables.
    pub const CLK_CONF: u32 = 0x78;
    /// ID; bit 31 (`UPDATE`) syncs the configuration and self-clears.
    pub const ID: u32 = 0x80;
}

/// APB clock used when `SCLK_SEL` is APB (the model's APB is 80 MHz).
pub const APB_HZ: u64 = 80_000_000;

/// `INT_RAW.RXFIFO_FULL` (bit 0): RX FIFO count > `RXFIFO_FULL_THRHD`.
pub const RXFIFO_FULL: u32 = 1 << 0;
/// `INT_RAW.TXFIFO_EMPTY` (bit 1).
pub const TXFIFO_EMPTY: u32 = 1 << 1;
/// `INT_RAW.RXFIFO_OVF` (bit 4): a received byte was lost.
pub const RXFIFO_OVF: u32 = 1 << 4;
/// `INT_RAW.RXFIFO_TOUT` (bit 8): RX timeout.
pub const RXFIFO_TOUT: u32 = 1 << 8;

const STATUS_IDLE: u32 = 0xe000_c000;

const CONF0_PARITY_EN: u32 = 1 << 1;
const CONF0_RXFIFO_RST: u32 = 1 << 17;
const CONF0_TXFIFO_RST: u32 = 1 << 18;
const CONF1_DIS_RX_DAT_OVF: u32 = 1 << 18;
const CONF1_RX_TOUT_EN: u32 = 1 << 21;

/// Configured registers, by offset, with their reset values.
const CONFIG_RESET: [(u32, u32); 9] = [
    (reg::INT_ENA, 0),
    (reg::CLKDIV, 0x2b6),
    (reg::CONF0, 0x1000_001c),
    (reg::CONF1, 0xc060),
    (reg::SLEEP_CONF, 0xf0),
    (reg::IDLE_CONF, 0x0004_0100),
    (reg::MEM_CONF, 0x000a_0012),
    (reg::CLK_CONF, 0x0370_1000),
    (reg::ID, 0x4000_0500),
];

/// UART0 configuration, interrupt state, receive FIFO and pending RX.
#[derive(Debug, Clone, PartialEq)]
pub struct Uart {
    config: [u32; CONFIG_RESET.len()],
    int_raw: u32,
    rx: VecDeque<u8>,
    pending: VecDeque<u8>,
    /// Arrival time of the next pending byte, if any.
    next_byte_at: Option<SimTime>,
    /// Time the most recent byte landed in the FIFO (for RX timeout).
    last_rx_at: Option<SimTime>,
    /// Timeout timer origin after a tout clear; `None` means `last_rx_at`.
    tout_from: Option<SimTime>,
    /// Last time [`Self::advance`] or [`Self::queue_rx`] was given.
    now: SimTime,
}

impl Default for Uart {
    fn default() -> Self {
        Self::new()
    }
}

impl Uart {
    /// Hardware RX FIFO depth (bytes).
    pub const RX_CAPACITY: usize = 128;

    /// Reset state.
    pub fn new() -> Self {
        Self {
            config: CONFIG_RESET.map(|(_, v)| v),
            int_raw: TXFIFO_EMPTY,
            rx: VecDeque::new(),
            pending: VecDeque::new(),
            next_byte_at: None,
            last_rx_at: None,
            tout_from: None,
            now: SimTime::ZERO,
        }
    }

    fn slot(off: u32) -> Option<usize> {
        CONFIG_RESET.iter().position(|&(o, _)| o == off)
    }

    fn cfg(&self, off: u32) -> u32 {
        Self::slot(off).map_or(0, |i| self.config[i])
    }

    fn status(&self) -> u32 {
        (STATUS_IDLE & !0x3ff) | (self.rx.len() as u32 & 0x3ff)
    }

    fn rxfifo_full_thrhd(&self) -> usize {
        (self.cfg(reg::CONF1) & 0x1ff) as usize
    }

    fn rx_tout_en(&self) -> bool {
        self.cfg(reg::CONF1) & CONF1_RX_TOUT_EN != 0
    }

    fn rx_tout_thrhd(&self) -> u32 {
        (self.cfg(reg::MEM_CONF) >> 16) & 0x3ff
    }

    fn dis_rx_ovf(&self) -> bool {
        self.cfg(reg::CONF1) & CONF1_DIS_RX_DAT_OVF != 0
    }

    /// Bits in one UART frame: start + data + optional parity + stop.
    pub fn bits_per_frame(&self) -> f64 {
        let conf0 = self.cfg(reg::CONF0);
        let data = match (conf0 >> 2) & 3 {
            0 => 5.0,
            1 => 6.0,
            2 => 7.0,
            _ => 8.0,
        };
        let parity = if conf0 & CONF0_PARITY_EN != 0 { 1.0 } else { 0.0 };
        let stop = match (conf0 >> 4) & 3 {
            2 => 1.5,
            3 => 2.0,
            _ => 1.0,
        };
        1.0 + data + parity + stop
    }

    /// Duration of one frame at the configured baud, if the baud is valid.
    pub fn frame_time(&self, apb_hz: u64) -> Option<SimTime> {
        let baud = self.baud(apb_hz)?;
        (baud > 0.0).then(|| SimTime::from_secs_f64(self.bits_per_frame() / baud))
    }

    fn bit_time(&self, apb_hz: u64) -> Option<SimTime> {
        let baud = self.baud(apb_hz)?;
        (baud > 0.0).then(|| SimTime::from_secs_f64(1.0 / baud))
    }

    fn tout_at(&self, apb_hz: u64) -> Option<SimTime> {
        if !self.rx_tout_en() || self.rx.is_empty() {
            return None;
        }
        let bits = self.rx_tout_thrhd();
        if bits == 0 {
            return None;
        }
        let dt = self.bit_time(apb_hz)?.0.saturating_mul(u64::from(bits));
        let from = self.tout_from.or(self.last_rx_at)?;
        Some(from.saturating_add(SimTime(dt)))
    }

    fn deposit(&mut self, byte: u8, at: SimTime) {
        if self.rx.len() == Self::RX_CAPACITY {
            if !self.dis_rx_ovf() {
                self.int_raw |= RXFIFO_OVF;
            }
            // The new byte is lost (TRM: RX FIFO overflow).
        } else {
            self.rx.push_back(byte);
        }
        self.last_rx_at = Some(at);
        self.tout_from = None;
        self.refresh_level_irqs();
    }

    fn refresh_level_irqs(&mut self) {
        if self.rx.len() > self.rxfifo_full_thrhd() {
            self.int_raw |= RXFIFO_FULL;
        } else {
            self.int_raw &= !RXFIFO_FULL;
        }
        self.int_raw |= TXFIFO_EMPTY;
        if self.rx.is_empty() {
            self.int_raw &= !RXFIFO_TOUT;
        }
    }

    /// Push host (or test) bytes into the RX FIFO instantly.
    ///
    /// Prefer [`Self::queue_rx`] when the arrival should take one frame time
    /// per byte at the configured baud.
    pub fn push_rx(&mut self, bytes: &[u8]) {
        for &b in bytes {
            self.deposit(b, self.now);
        }
    }

    /// Queue host bytes to arrive one frame at a time from `now`.
    ///
    /// If the baud divider is invalid, the bytes are deposited at once so
    /// they are not stuck in the shifter.
    pub fn queue_rx(&mut self, now: SimTime, bytes: &[u8], apb_hz: u64) {
        if bytes.is_empty() {
            return;
        }
        self.now = now.max(self.now);
        self.pending.extend(bytes.iter().copied());
        if self.next_byte_at.is_none() {
            match self.frame_time(apb_hz) {
                Some(dt) => self.next_byte_at = Some(self.now.saturating_add(dt)),
                None => {
                    while let Some(b) = self.pending.pop_front() {
                        self.deposit(b, self.now);
                    }
                }
            }
        }
    }

    /// Deliver due RX bytes and raise a timeout that has elapsed by `now`.
    pub fn advance(&mut self, now: SimTime, apb_hz: u64) {
        self.now = now.max(self.now);
        while let Some(at) = self.next_byte_at
            && at <= self.now
        {
            let Some(b) = self.pending.pop_front() else {
                self.next_byte_at = None;
                break;
            };
            self.deposit(b, at);
            self.next_byte_at = if self.pending.is_empty() {
                None
            } else {
                self.frame_time(apb_hz).map(|dt| at.saturating_add(dt))
            };
        }
        if let Some(at) = self.tout_at(apb_hz)
            && at <= self.now
        {
            self.int_raw |= RXFIFO_TOUT;
        }
    }

    /// Next time a queued byte arrives or an RX timeout should fire.
    pub fn next_event(&self, apb_hz: u64) -> Option<SimTime> {
        let tout = self
            .tout_at(apb_hz)
            .filter(|&at| at > self.now && self.int_raw & RXFIFO_TOUT == 0);
        [self.next_byte_at, tout].into_iter().flatten().min()
    }

    /// Pop one RX byte, or `0` if the FIFO is empty.
    pub fn pop_rx(&mut self) -> u8 {
        let b = self.rx.pop_front().unwrap_or(0);
        self.refresh_level_irqs();
        if self.rx.is_empty() {
            self.tout_from = None;
        }
        b
    }

    /// Masked interrupt request (any `INT_ST` bit).
    pub fn irq(&self) -> bool {
        self.int_raw & self.cfg(reg::INT_ENA) != 0
    }

    /// Read a register other than the FIFO; `None` if not modelled.
    ///
    /// FIFO reads go through [`Self::pop_rx`] from the bus.
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            reg::FIFO => Some(0),
            reg::INT_RAW => Some(self.int_raw),
            reg::INT_ST => Some(self.int_raw & self.cfg(reg::INT_ENA)),
            reg::INT_CLR => Some(0),
            reg::STATUS => Some(self.status()),
            reg::FSM_STATUS => Some(0),
            _ => Self::slot(off).map(|i| self.config[i]),
        }
    }

    /// Write a register other than the FIFO; `None` if not modelled.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        match off {
            reg::INT_CLR => {
                self.int_raw &= !v;
                if v & RXFIFO_TOUT != 0 {
                    self.tout_from = Some(self.now);
                }
                self.refresh_level_irqs();
            }
            reg::INT_RAW | reg::INT_ST | reg::STATUS | reg::FSM_STATUS => {}
            reg::ID => self.config[Self::slot(off)?] = v & !(1 << 31),
            reg::CONF0 => {
                self.config[Self::slot(off)?] = v & !(CONF0_RXFIFO_RST | CONF0_TXFIFO_RST);
                if v & CONF0_RXFIFO_RST != 0 {
                    self.rx.clear();
                    self.pending.clear();
                    self.next_byte_at = None;
                    self.last_rx_at = None;
                    self.tout_from = None;
                    self.int_raw &= !(RXFIFO_FULL | RXFIFO_TOUT);
                }
                self.refresh_level_irqs();
            }
            _ => {
                self.config[Self::slot(off)?] = v;
                self.refresh_level_irqs();
            }
        }
        Some(())
    }

    /// The configured baud rate in Hz: the core clock (`SCLK_SEL` source,
    /// with `apb_hz` for APB, divided by `SCLK_DIV_NUM + 1 + A/B`) over
    /// `CLKDIV + FRAG/16`. `None` if the divider or the source is invalid.
    pub fn baud(&self, apb_hz: u64) -> Option<f64> {
        let clk = self.cfg(reg::CLK_CONF);
        let src = match (clk >> 20) & 3 {
            1 => apb_hz as f64,
            2 => 17.5e6,
            3 => 40e6,
            _ => return None,
        };
        let (b, a, num) = (clk & 0x3f, (clk >> 6) & 0x3f, (clk >> 12) & 0xff);
        let frac = if b == 0 { 0.0 } else { f64::from(a) / f64::from(b) };
        let sclk = src / (f64::from(num) + 1.0 + frac);
        let div = self.cfg(reg::CLKDIV);
        let div = f64::from(div & 0xfff) + f64::from((div >> 20) & 0xf) / 16.0;
        (div >= 1.0).then(|| sclk / div)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uart_115200() -> Uart {
        let mut u = Uart::new();
        // XTAL / 1 core clock, divider 347 + 3/16: 115 200 baud (to 0.01 %).
        u.write(reg::CLK_CONF, 0x0370_0000).unwrap();
        u.write(reg::CLKDIV, 3 << 20 | 347).unwrap();
        u
    }

    #[test]
    fn reset_values_and_tx_status() {
        let mut u = Uart::new();
        assert_eq!(u.read(reg::CONF0), Some(0x1000_001c));
        assert_eq!(u.read(reg::CLK_CONF), Some(0x0370_1000));
        assert_eq!(u.read(reg::STATUS), Some(0xe000_c000), "empty FIFOs");
        assert_eq!(u.read(reg::INT_RAW), Some(TXFIFO_EMPTY));
        u.write(reg::INT_ENA, TXFIFO_EMPTY).unwrap();
        assert_eq!(u.read(reg::INT_ST), Some(TXFIFO_EMPTY));
        u.write(reg::INT_CLR, u32::MAX).unwrap();
        assert_eq!(u.read(reg::INT_RAW), Some(TXFIFO_EMPTY), "the FIFO is still empty");
        u.write(reg::ID, 0xc000_0500).unwrap();
        assert_eq!(u.read(reg::ID), Some(0x4000_0500), "UPDATE self-clears");
        assert_eq!(u.read(0x28), None);
    }

    #[test]
    fn baud_rate_from_clock_and_divider() {
        let u = uart_115200();
        let baud = u.baud(APB_HZ).unwrap();
        assert!((baud - 115_200.0).abs() < 12.0, "{baud}");
        let mut u = uart_115200();
        // APB core clock, divider 694 + 7/16.
        u.write(reg::CLK_CONF, 0x0310_0000).unwrap();
        u.write(reg::CLKDIV, 7 << 20 | 694).unwrap();
        assert!((u.baud(APB_HZ).unwrap() - 115_200.0).abs() < 12.0);
        u.write(reg::CLKDIV, 0).unwrap();
        assert_eq!(u.baud(APB_HZ), None);
    }

    #[test]
    fn rx_fifo_push_pop_and_status_count() {
        let mut u = Uart::new();
        u.push_rx(b"ab");
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, 2, "RXFIFO_CNT");
        assert_eq!(u.pop_rx(), b'a');
        assert_eq!(u.pop_rx(), b'b');
        assert_eq!(u.pop_rx(), 0);
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, 0);
    }

    #[test]
    fn rx_fifo_overflow_drops_the_new_byte_and_sets_ovf() {
        let mut u = Uart::new();
        let mut data = vec![0u8; Uart::RX_CAPACITY + 3];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        u.push_rx(&data);
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, Uart::RX_CAPACITY as u32);
        assert_eq!(u.read(reg::INT_RAW).unwrap() & RXFIFO_OVF, RXFIFO_OVF);
        assert_eq!(u.pop_rx(), data[0], "oldest bytes are kept");
    }

    #[test]
    fn queue_rx_arrives_one_frame_at_a_time() {
        let mut u = uart_115200();
        let frame = u.frame_time(APB_HZ).unwrap();
        assert!((frame.as_secs_f64() - 10.0 / 115_200.0).abs() < 1e-8);
        u.queue_rx(SimTime::ZERO, b"AB", APB_HZ);
        u.advance(SimTime::from_us(1), APB_HZ);
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, 0, "still shifting");
        u.advance(frame, APB_HZ);
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, 1);
        assert_eq!(u.pop_rx(), b'A');
        u.advance(frame.saturating_add(frame), APB_HZ);
        assert_eq!(u.pop_rx(), b'B');
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, 0);
    }

    #[test]
    fn rxfifo_full_tracks_the_threshold_and_int_st() {
        let mut u = Uart::new();
        // Full interrupt when count > 1.
        u.write(reg::CONF1, 1).unwrap();
        u.write(reg::INT_ENA, RXFIFO_FULL).unwrap();
        u.push_rx(b"a");
        assert_eq!(u.read(reg::INT_RAW).unwrap() & RXFIFO_FULL, 0);
        assert!(!u.irq());
        u.push_rx(b"b");
        assert_eq!(u.read(reg::INT_ST).unwrap() & RXFIFO_FULL, RXFIFO_FULL);
        assert!(u.irq());
        u.pop_rx();
        assert_eq!(u.read(reg::INT_RAW).unwrap() & RXFIFO_FULL, 0);
        assert!(!u.irq());
    }

    #[test]
    fn rxfifo_tout_fires_after_the_configured_idle_and_clears() {
        let mut u = uart_115200();
        u.write(reg::CONF1, u.read(reg::CONF1).unwrap() | CONF1_RX_TOUT_EN)
            .unwrap();
        u.write(reg::INT_ENA, RXFIFO_TOUT).unwrap();
        // Default MEM_CONF.RX_TOUT_THRHD is 10 bit times.
        u.queue_rx(SimTime::ZERO, b"X", APB_HZ);
        let frame = u.frame_time(APB_HZ).unwrap();
        u.advance(frame, APB_HZ);
        assert_eq!(u.pop_rx(), b'X');
        // Put the byte back for a leftover FIFO (pop consumed it).
        u.push_rx(b"X");
        let bit = u.bit_time_for_test();
        let tout = SimTime(bit.0 * 10);
        u.advance(u.last_rx_at.unwrap().saturating_add(SimTime(tout.0 - 1)), APB_HZ);
        assert_eq!(u.read(reg::INT_RAW).unwrap() & RXFIFO_TOUT, 0, "not yet");
        u.advance(u.last_rx_at.unwrap().saturating_add(tout), APB_HZ);
        assert_eq!(u.read(reg::INT_ST).unwrap() & RXFIFO_TOUT, RXFIFO_TOUT);
        u.write(reg::INT_CLR, RXFIFO_TOUT).unwrap();
        assert_eq!(u.read(reg::INT_RAW).unwrap() & RXFIFO_TOUT, 0);
    }

    impl Uart {
        fn bit_time_for_test(&self) -> SimTime {
            self.bit_time(APB_HZ).unwrap()
        }
    }

    #[test]
    fn conf0_rxfifo_rst_clears_the_fifo() {
        let mut u = Uart::new();
        u.push_rx(b"xy");
        u.write(reg::CONF0, 0x1000_001c | CONF0_RXFIFO_RST).unwrap();
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, 0);
        assert_eq!(u.read(reg::CONF0), Some(0x1000_001c), "RST self-clears");
    }
}
