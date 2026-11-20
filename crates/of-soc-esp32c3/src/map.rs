//! ESP32-C3 address map (TRM chapter 3, "System and Memory").

/// Internal ROM, instruction bus (ROM0 + ROM1, 384 KiB).
pub const IROM_INTERNAL: (u32, u32) = (0x4000_0000, 0x6_0000);
/// Internal ROM, data bus alias of ROM1 (128 KiB).
pub const DROM_INTERNAL: (u32, u32) = (0x3ff0_0000, 0x2_0000);
/// SRAM through the instruction bus. The first 16 KiB are the ICache.
pub const IRAM: (u32, u32) = (0x4037_c000, 0x6_4000);
/// SRAM through the data bus. DRAM address = IRAM address - 0x0070_0000.
pub const DRAM: (u32, u32) = (0x3fc7_c000, 0x6_4000);
/// Flash through the instruction cache (8 MiB window).
pub const IROM_FLASH: (u32, u32) = (0x4200_0000, 0x80_0000);
/// Flash through the data cache (8 MiB window).
pub const DROM_FLASH: (u32, u32) = (0x3c00_0000, 0x80_0000);
/// RTC fast memory (8 KiB), same bytes on both buses.
pub const RTC_FAST: (u32, u32) = (0x5000_0000, 0x2000);
/// Peripheral register space.
pub const PERIPHERALS: (u32, u32) = (0x6000_0000, 0xd_1000);

/// UART0 registers.
pub const UART0_BASE: u32 = 0x6000_0000;
/// SPI1 memory controller (software flash commands).
pub const SPI1_BASE: u32 = 0x6000_2000;
/// SPI0 memory controller (cache side).
pub const SPI0_BASE: u32 = 0x6000_3000;
/// GPIO (and GPIO matrix) registers.
pub const GPIO_BASE: u32 = 0x6000_4000;
/// RTC_CNTL registers (RTC watchdog, super watchdog).
pub const RTC_CNTL_BASE: u32 = 0x6000_8000;
/// eFuse controller registers, inside the RTC_CNTL block.
pub const EFUSE_BASE: u32 = 0x6000_8800;
/// Offset of the eFuse controller in the RTC_CNTL block.
pub const EFUSE_OFFSET: u32 = EFUSE_BASE - RTC_CNTL_BASE;
/// IO MUX registers.
pub const IO_MUX_BASE: u32 = 0x6000_9000;
/// LED PWM controller (LEDC) registers.
pub const LEDC_BASE: u32 = 0x6001_9000;
/// Timer group 0 registers.
pub const TIMG0_BASE: u32 = 0x6001_f000;
/// Timer group 1 registers.
pub const TIMG1_BASE: u32 = 0x6002_0000;
/// APB_CTRL (SYSCON) registers, including `RNG_DATA_REG`.
pub const APB_CTRL_BASE: u32 = 0x6002_6000;
/// SYSTIMER registers.
pub const SYSTIMER_BASE: u32 = 0x6002_3000;
/// SHA accelerator registers.
pub const SHA_BASE: u32 = 0x6003_b000;
/// SAR ADC digital controller (APB_SARADC) registers.
pub const APB_SARADC_BASE: u32 = 0x6004_0000;
/// USB Serial/JTAG controller registers.
pub const USB_SERIAL_JTAG_BASE: u32 = 0x6004_3000;
/// SYSTEM registers (clocks, peripheral clock gates and resets).
pub const SYSTEM_BASE: u32 = 0x600c_0000;
/// Interrupt matrix and CPU interrupt controller registers.
pub const INTERRUPT_BASE: u32 = 0x600c_2000;
/// External memory (cache) controller registers.
pub const EXTMEM_BASE: u32 = 0x600c_4000;
/// Cache MMU table (128 entries).
pub const MMU_TABLE_BASE: u32 = 0x600c_5000;
/// Size of one peripheral's register block.
pub const BLOCK: u32 = 0x1000;

/// Whether `addr..addr+len` lies in `region`.
pub fn within(region: (u32, u32), addr: u32, len: u32) -> bool {
    addr >= region.0 && u64::from(addr - region.0) + u64::from(len) <= u64::from(region.1)
}
