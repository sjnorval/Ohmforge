//! SYSTEM registers (TRM chapter "System Registers"):
//!
//! - CPU clock: `SYSTEM_CPU_PER_CONF_REG` (`0x08`) and
//!   `SYSTEM_SYSCLK_CONF_REG` (`0x58`).
//! - Software / cross-core interrupts: `SYSTEM_CPU_INTR_FROM_CPU_n_REG`
//!   (`0x28`–`0x34`), used by FreeRTOS yield on the C3 (unicore still
//!   triggers `FROM_CPU_INTR0`).
//!
//! `SOC_CLK_SEL` picks the CPU clock source: 0 XTAL (40 MHz) divided by
//! `PRE_DIV_CNT + 1`, 1 the PLL (`CPUPERIOD_SEL` 0 gives 80 MHz, 1 gives
//! 160 MHz, with the 320 or 480 MHz PLL alike), 2 RC_FAST (17.5 MHz)
//! divided by `PRE_DIV_CNT + 1`. At reset that is XTAL / 2 (20 MHz); the ROM
//! switches to XTAL / 1 (40 MHz), the ESP-IDF bootloader to the PLL at
//! 80 MHz and esp-hal to 160 MHz. [`System::cpu_hz`] gives the result;
//! [`crate::Esp32c3`] retimes the hart when it changes. The PLL locks at
//! once (its regi2c calibration registers are not modelled), and APB
//! peripherals keep their 80 MHz APB timing whatever the CPU clock (on
//! hardware APB follows the CPU clock when it is not on the PLL).
//! `CLK_XTAL_FREQ` reads 40. Other SYSTEM registers are not modelled.

/// Register offsets from [`crate::map::SYSTEM_BASE`].
pub mod reg {
    /// CPU clock: `CPUPERIOD_SEL` bits 1:0, `PLL_FREQ_SEL` bit 2.
    pub const CPU_PER_CONF: u32 = 0x08;
    /// Software interrupt 0 (FreeRTOS yield); bit 0 sets source 50.
    pub const CPU_INTR_FROM_CPU_0: u32 = 0x28;
    /// Software interrupt 1; bit 0 sets source 51.
    pub const CPU_INTR_FROM_CPU_1: u32 = 0x2c;
    /// Software interrupt 2; bit 0 sets source 52.
    pub const CPU_INTR_FROM_CPU_2: u32 = 0x30;
    /// Software interrupt 3; bit 0 sets source 53.
    pub const CPU_INTR_FROM_CPU_3: u32 = 0x34;
    /// SoC clock: `PRE_DIV_CNT` bits 9:0, `SOC_CLK_SEL` bits 11:10,
    /// `CLK_XTAL_FREQ` bits 18:12 (read-only).
    pub const SYSCLK_CONF: u32 = 0x58;
}

const XTAL_HZ: u64 = 40_000_000;
const RC_FAST_HZ: u64 = 17_500_000;
/// `CLK_XTAL_FREQ` (read-only): 40 MHz.
const CLK_XTAL_FREQ: u32 = 40 << 12;

/// The CPU clock configuration and software-interrupt triggers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct System {
    cpu_per_conf: u32,
    sysclk_conf: u32,
    /// `SYSTEM_CPU_INTR_FROM_CPU_n` bit 0 for n = 0..3.
    from_cpu: [u32; 4],
}

impl Default for System {
    fn default() -> Self {
        Self::new()
    }
}

impl System {
    /// Reset state: XTAL / 2.
    pub fn new() -> Self {
        Self {
            cpu_per_conf: 0x0c,
            sysclk_conf: 0x01,
            from_cpu: [0; 4],
        }
    }

    /// The state ESP-IDF and esp-hal apps run in: the 480 MHz PLL with
    /// the CPU at 160 MHz (for boots that skip the ROM and bootloader).
    pub fn pll_160mhz() -> Self {
        Self {
            cpu_per_conf: 0x0d,
            sysclk_conf: 1 << 10,
            from_cpu: [0; 4],
        }
    }

    /// The CPU clock frequency.
    pub fn cpu_hz(&self) -> u64 {
        let div = u64::from(self.sysclk_conf & 0x3ff) + 1;
        match (self.sysclk_conf >> 10) & 3 {
            0 => XTAL_HZ / div,
            1 if self.cpu_per_conf & 3 == 0 => 80_000_000,
            1 => 160_000_000,
            _ => RC_FAST_HZ / div,
        }
    }

    /// Pending software-interrupt bits for sources 50–53 (`FROM_CPU_INTRn`).
    pub fn from_cpu_irq(&self) -> u8 {
        let mut bits = 0u8;
        for (i, &v) in self.from_cpu.iter().enumerate() {
            if v & 1 != 0 {
                bits |= 1 << i;
            }
        }
        bits
    }

    /// Read a register; `None` if not modelled.
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            reg::CPU_PER_CONF => Some(self.cpu_per_conf),
            reg::CPU_INTR_FROM_CPU_0 => Some(self.from_cpu[0]),
            reg::CPU_INTR_FROM_CPU_1 => Some(self.from_cpu[1]),
            reg::CPU_INTR_FROM_CPU_2 => Some(self.from_cpu[2]),
            reg::CPU_INTR_FROM_CPU_3 => Some(self.from_cpu[3]),
            reg::SYSCLK_CONF => Some(self.sysclk_conf | CLK_XTAL_FREQ),
            _ => None,
        }
    }

    /// Write a register; `None` if not modelled.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        match off {
            reg::CPU_PER_CONF => self.cpu_per_conf = v & 0xff,
            reg::CPU_INTR_FROM_CPU_0 => self.from_cpu[0] = v & 1,
            reg::CPU_INTR_FROM_CPU_1 => self.from_cpu[1] = v & 1,
            reg::CPU_INTR_FROM_CPU_2 => self.from_cpu[2] = v & 1,
            reg::CPU_INTR_FROM_CPU_3 => self.from_cpu[3] = v & 1,
            // CLK_XTAL_FREQ is read-only; bit 19 (CLK_DIV_EN) is kept.
            reg::SYSCLK_CONF => self.sysclk_conf = v & 0x8_0fff,
            _ => return None,
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_clock_follows_source_and_divider() {
        let mut s = System::new();
        assert_eq!(s.cpu_hz(), 20_000_000, "XTAL / 2 at reset");
        assert_eq!(s.read(reg::SYSCLK_CONF), Some(0x2_8001), "CLK_XTAL_FREQ reads 40");
        s.write(reg::SYSCLK_CONF, 0).unwrap();
        assert_eq!(s.cpu_hz(), 40_000_000, "the ROM's XTAL / 1");
        // The bootloader: 480 MHz PLL, CPUPERIOD_SEL 0.
        s.write(reg::CPU_PER_CONF, 0x4).unwrap();
        s.write(reg::SYSCLK_CONF, 0x400).unwrap();
        assert_eq!(s.cpu_hz(), 80_000_000);
        s.write(reg::CPU_PER_CONF, 0x5).unwrap();
        assert_eq!(s.cpu_hz(), 160_000_000);
        assert_eq!(System::pll_160mhz().cpu_hz(), 160_000_000);
        s.write(reg::SYSCLK_CONF, 0x803).unwrap();
        assert_eq!(s.cpu_hz(), 17_500_000 / 4, "RC_FAST / 4");
        assert_eq!(s.read(0x10), None);
    }

    #[test]
    fn from_cpu_software_interrupt_bit() {
        let mut s = System::new();
        assert_eq!(s.from_cpu_irq(), 0);
        s.write(reg::CPU_INTR_FROM_CPU_0, 1).unwrap();
        assert_eq!(s.read(reg::CPU_INTR_FROM_CPU_0), Some(1));
        assert_eq!(s.from_cpu_irq(), 0b0001);
        s.write(reg::CPU_INTR_FROM_CPU_0, 0).unwrap();
        assert_eq!(s.from_cpu_irq(), 0);
        s.write(reg::CPU_INTR_FROM_CPU_3, 1).unwrap();
        assert_eq!(s.from_cpu_irq(), 0b1000);
    }
}
