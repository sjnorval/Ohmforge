//! UART0 (TRM chapter 26): transmit capture, receive FIFO, and configuration
//! registers.
//!
//! Bytes written to `UART_FIFO` are captured instantly: the TX FIFO never
//! fills, so `UART_STATUS.TXFIFO_CNT` reads 0, `INT_RAW.TXFIFO_EMPTY` stays
//! set and `UART_FSM_STATUS` reads idle. Host (or test) bytes pushed with
//! [`Uart::push_rx`] land in a 128-byte RX FIFO instantly; FIFO reads pop
//! them and `STATUS.RXFIFO_CNT` reflects the depth. Overfill drops the
//! oldest bytes. The configuration registers (`INT_ENA`, `CLKDIV`, `CONF0`,
//! `CONF1`, `SLEEP_CONF`, `IDLE_CONF`, `MEM_CONF`, `CLK_CONF`, `ID`) hold
//! what is written, with the TRM reset values, and [`Uart::baud`] works out
//! the configured baud rate from `CLK_CONF` and `CLKDIV`. Baud-rate timing
//! of TX/RX and UART interrupts are not modelled yet.

use std::collections::VecDeque;

/// Register offsets from [`crate::map::UART0_BASE`].
pub mod reg {
    /// TX/RX FIFO.
    pub const FIFO: u32 = 0x00;
    /// Raw interrupt status (`TXFIFO_EMPTY` is bit 1).
    pub const INT_RAW: u32 = 0x04;
    /// Masked interrupt status.
    pub const INT_ST: u32 = 0x08;
    /// Interrupt enable.
    pub const INT_ENA: u32 = 0x0c;
    /// Interrupt clear (write-only).
    pub const INT_CLR: u32 = 0x10;
    /// Baud divider: integer part bits 11:0, `FRAG` (sixteenths) bits 23:20.
    pub const CLKDIV: u32 = 0x14;
    /// Status: `RXFIFO_CNT` bits 9:0, `TXFIFO_CNT` bits 25:16.
    pub const STATUS: u32 = 0x1c;
    /// Frame format and control.
    pub const CONF0: u32 = 0x20;
    /// FIFO thresholds.
    pub const CONF1: u32 = 0x24;
    /// Light-sleep wake-up threshold.
    pub const SLEEP_CONF: u32 = 0x38;
    /// Idle and TX break timing.
    pub const IDLE_CONF: u32 = 0x48;
    /// FIFO memory allocation.
    pub const MEM_CONF: u32 = 0x60;
    /// Transmit and receive state machines (read-only).
    pub const FSM_STATUS: u32 = 0x6c;
    /// Core clock: `SCLK_DIV_B` 5:0, `SCLK_DIV_A` 11:6, `SCLK_DIV_NUM`
    /// 19:12, `SCLK_SEL` 21:20 (1 APB, 2 RC_FAST, 3 XTAL), enables.
    pub const CLK_CONF: u32 = 0x78;
    /// ID; bit 31 (`UPDATE`) syncs the configuration and self-clears.
    pub const ID: u32 = 0x80;
}

/// `INT_RAW.TXFIFO_EMPTY`.
const TXFIFO_EMPTY: u32 = 1 << 1;
/// `STATUS` with empty FIFOs and idle-high lines (TXD, RTSN, DTRN, RXD,
/// CTSN). `RXFIFO_CNT` (bits 9:0) is overlaid at read time.
const STATUS_IDLE: u32 = 0xe000_c000;

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

/// UART0 configuration, interrupt state, and receive FIFO.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Uart {
    config: [u32; CONFIG_RESET.len()],
    int_raw: u32,
    rx: VecDeque<u8>,
}

impl Default for Uart {
    fn default() -> Self {
        Self::new()
    }
}

impl Uart {
    /// Hardware RX FIFO depth (bytes). Overfill drops the oldest byte.
    pub const RX_CAPACITY: usize = 128;

    /// Reset state.
    pub fn new() -> Self {
        Self {
            config: CONFIG_RESET.map(|(_, v)| v),
            int_raw: TXFIFO_EMPTY,
            rx: VecDeque::new(),
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

    /// Push host (or test) bytes into the RX FIFO instantly.
    ///
    /// When the FIFO is full, the oldest bytes are dropped so the newest
    /// input is kept.
    pub fn push_rx(&mut self, bytes: &[u8]) {
        for &b in bytes {
            if self.rx.len() == Self::RX_CAPACITY {
                self.rx.pop_front();
            }
            self.rx.push_back(b);
        }
    }

    /// Pop one RX byte, or `0` if the FIFO is empty.
    pub fn pop_rx(&mut self) -> u8 {
        self.rx.pop_front().unwrap_or(0)
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
            // The TX FIFO is always empty again at once.
            reg::INT_CLR => self.int_raw = (self.int_raw & !v) | TXFIFO_EMPTY,
            reg::INT_RAW | reg::INT_ST | reg::STATUS | reg::FSM_STATUS => {}
            reg::ID => self.config[Self::slot(off)?] = v & !(1 << 31),
            _ => self.config[Self::slot(off)?] = v,
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
        let mut u = Uart::new();
        // XTAL / 1 core clock, divider 347 + 3/16: 115 200 baud (to 0.01 %).
        u.write(reg::CLK_CONF, 0x0370_0000).unwrap();
        u.write(reg::CLKDIV, 3 << 20 | 347).unwrap();
        let baud = u.baud(80_000_000).unwrap();
        assert!((baud - 115_200.0).abs() < 12.0, "{baud}");
        // APB core clock, divider 694 + 7/16.
        u.write(reg::CLK_CONF, 0x0310_0000).unwrap();
        u.write(reg::CLKDIV, 7 << 20 | 694).unwrap();
        assert!((u.baud(80_000_000).unwrap() - 115_200.0).abs() < 12.0);
        u.write(reg::CLKDIV, 0).unwrap();
        assert_eq!(u.baud(80_000_000), None);
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
    fn rx_fifo_cap_drops_oldest() {
        let mut u = Uart::new();
        let mut data = vec![0u8; Uart::RX_CAPACITY + 3];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        u.push_rx(&data);
        assert_eq!(u.read(reg::STATUS).unwrap() & 0x3ff, Uart::RX_CAPACITY as u32);
        assert_eq!(u.pop_rx(), data[3]);
    }
}
