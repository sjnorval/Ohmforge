//! USB Serial/JTAG controller (`0x6004_3000`), console side only.
//!
//! The ROM and ESP-IDF print to it as well as to UART0. Bytes written to
//! `EP1` are captured ([`UsbSerialJtag::take_tx`]); `EP1_CONF` always
//! reports room in the IN endpoint (`SERIAL_IN_EP_DATA_FREE`) and no data
//! from the host, as if a host were reading. USB itself is not modelled;
//! other registers read back the last write.

/// Register offsets.
pub mod reg {
    /// Endpoint 1 data FIFO.
    pub const EP1: u32 = 0x00;
    /// Endpoint 1 configuration and status.
    pub const EP1_CONF: u32 = 0x04;
}

/// `EP1_CONF`: the IN endpoint can take data.
pub const SERIAL_IN_EP_DATA_FREE: u32 = 1 << 1;

const REGS: usize = 0x100 / 4;

/// The controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UsbSerialJtag {
    regs: Vec<u32>,
    tx: Vec<u8>,
}

impl Default for UsbSerialJtag {
    fn default() -> Self {
        Self::new()
    }
}

impl UsbSerialJtag {
    /// Reset state.
    pub fn new() -> Self {
        Self {
            regs: vec![0; REGS],
            tx: Vec::new(),
        }
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            reg::EP1 => Some(0),
            reg::EP1_CONF => Some(SERIAL_IN_EP_DATA_FREE),
            _ => self.regs.get((off / 4) as usize).copied(),
        }
    }

    /// Write a register.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        match off {
            reg::EP1 => self.tx.push(v as u8),
            reg::EP1_CONF => {}
            _ => *self.regs.get_mut((off / 4) as usize)? = v,
        }
        Some(())
    }

    /// Bytes written to the IN endpoint since the last call.
    pub fn take_tx(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.tx)
    }

    /// Everything written so far that has not been taken.
    pub fn tx(&self) -> &[u8] {
        &self.tx
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn captures_console_bytes() {
        let mut u = UsbSerialJtag::new();
        assert_eq!(u.read(reg::EP1_CONF), Some(SERIAL_IN_EP_DATA_FREE));
        u.write(reg::EP1, u32::from(b'h')).unwrap();
        u.write(reg::EP1, u32::from(b'i')).unwrap();
        u.write(reg::EP1_CONF, 1).unwrap(); // WR_DONE: flush
        assert_eq!(u.take_tx(), b"hi");
        assert!(u.tx().is_empty());
    }
}
