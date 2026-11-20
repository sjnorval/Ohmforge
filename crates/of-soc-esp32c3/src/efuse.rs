//! eFuse controller (`0x6000_8800`; register offsets as in ESP-IDF's
//! `efuse_reg.h`).
//!
//! Models a chip of wafer revision v0.3 (the ECO3 die whose ROM Ohmforge
//! loads) with the factory calibration of a production part and nothing
//! else programmed: flash encryption, secure boot and download-mode
//! restrictions are all off. The calibration (BLOCK1 analog trims, BLOCK2
//! with `BLK_VERSION_MAJOR` 1: temperature sensor, ADC1 init codes and
//! calibration voltages) is the example chip from Espressif's esptool
//! `espefuse summary` documentation for the ESP32-C3, so esp-hal and
//! ESP-IDF find eFuse calibration as on a real module (see
//! [`crate::saradc`]). `EFUSE_CMD_REG` read and program commands finish at
//! once (the bit clears); programming does not burn anything. Other
//! registers read back the last write.

/// Register offsets from [`crate::map::EFUSE_BASE`].
pub mod reg {
    /// BLOCK1 (MAC, SPI pads, system data) word 0.
    pub const RD_MAC_SPI_SYS_0: u32 = 0x044;
    /// MAC, SPI pad configuration and wafer version (minor, low bits 20:18).
    pub const RD_MAC_SPI_SYS_3: u32 = 0x050;
    /// BLOCK2 (system data part 1: calibration) word 0.
    pub const RD_SYS_PART1_DATA0: u32 = 0x05c;
    /// Controller state (bits 3:0; 1 = idle).
    pub const STATUS: u32 = 0x1d0;
    /// Read (bit 0) and program (bit 1) commands.
    pub const CMD: u32 = 0x1d4;
    /// Version register.
    pub const DATE: u32 = 0x1fc;
}

const REGS: usize = 0x200 / 4;
/// Wafer version minor 3 (`EFUSE_WAFER_VERSION_MINOR_LO`).
const WAFER_V0_3: u32 = 3 << 18;

/// eFuse calibration version of BLOCK2 (`BLK_VERSION_MAJOR`): 1 means
/// ADC and temperature-sensor calibration are present.
pub const BLK_VERSION_MAJOR: u32 = 1;
/// `ADC1_INIT_CODE_ATTEN0..3` (10 bits each; esp-hal and ESP-IDF add 1000).
pub const ADC1_INIT_CODE: [u32; 4] = [0b01_1000_0110, 0b10_0001_1011, 0b10_0011_1010, 0b10_1010_1110];
/// `ADC1_CAL_VOL_ATTEN0..3`: the code the ADC read for the calibration
/// voltage (400, 550, 750 and 1370 mV) is 2000 plus this value, whose bit 9
/// is the sign (magnitude in bits 8:0).
pub const ADC1_CAL_VOL: [u32; 4] = [0b10_0011_0101, 0b00_0000_1101, 0b10_0010_0110, 0b10_0100_0111];

/// Field positions (block, first bit, width) as in esp-hal's
/// `efuse/esp32c3/fields.rs` (ESP-IDF `esp_efuse_table.csv`), with the
/// values programmed.
const CALIBRATION: [(u8, u32, u32, u32); 17] = [
    (1, 135, 7, 0b100_1001),      // K_RTC_LDO
    (1, 142, 7, 0b101_0000),      // K_DIG_LDO
    (1, 149, 8, 0x8a),            // V_RTC_DBIAS20
    (1, 157, 8, 0x93),            // V_DIG_DBIAS20
    (1, 165, 5, 0b1_0111),        // DIG_DBIAS_HVT
    (1, 170, 10, 0b01_1111_0100), // THRES_HVT
    (2, 128, 2, BLK_VERSION_MAJOR),
    (2, 131, 9, 0b1_0100_1000), // TEMP_CALIB
    (2, 140, 8, 0x4e),          // OCODE
    (2, 148, 10, ADC1_INIT_CODE[0]),
    (2, 158, 10, ADC1_INIT_CODE[1]),
    (2, 168, 10, ADC1_INIT_CODE[2]),
    (2, 178, 10, ADC1_INIT_CODE[3]),
    (2, 188, 10, ADC1_CAL_VOL[0]),
    (2, 198, 10, ADC1_CAL_VOL[1]),
    (2, 208, 10, ADC1_CAL_VOL[2]),
    (2, 218, 10, ADC1_CAL_VOL[3]),
];

/// The ADC1 code for the calibration voltage at attenuation `atten`
/// (0..=3), as esp-hal decodes `ADC1_CAL_VOL_ATTENx`.
pub fn adc1_cal_code(atten: usize) -> u32 {
    let v = ADC1_CAL_VOL[atten & 3];
    if v & (1 << 9) != 0 {
        2000 - (v & 0x1ff)
    } else {
        2000 + v
    }
}

/// The eFuse controller.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Efuse {
    regs: Vec<u32>,
}

impl Default for Efuse {
    fn default() -> Self {
        Self::new()
    }
}

impl Efuse {
    /// A v0.3 chip with factory calibration only.
    pub fn new() -> Self {
        let mut regs = vec![0; REGS];
        regs[(reg::RD_MAC_SPI_SYS_3 / 4) as usize] = WAFER_V0_3;
        for (block, start, width, value) in CALIBRATION {
            let base = if block == 1 {
                reg::RD_MAC_SPI_SYS_0
            } else {
                reg::RD_SYS_PART1_DATA0
            } / 4;
            for i in 0..width {
                let bit = start + i;
                regs[(base + bit / 32) as usize] |= ((value >> i) & 1) << (bit % 32);
            }
        }
        regs[(reg::STATUS / 4) as usize] = 1;
        regs[(reg::DATE / 4) as usize] = 0x0200_7200;
        Self { regs }
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        self.regs.get((off / 4) as usize).copied()
    }

    /// Write a register. Read-out and status registers ignore writes.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        let slot = self.regs.get_mut((off / 4) as usize)?;
        match off {
            // Program buffer (0x00..0x2c) and configuration (0x1c8..) are writable.
            0x000..0x02c | 0x1c8 | 0x1cc | 0x1d8..=0x1f8 => *slot = v,
            reg::CMD => *slot = v & !0b11,
            _ => {}
        }
        Some(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_chip_with_wafer_version_and_instant_commands() {
        let mut e = Efuse::new();
        assert_eq!(e.read(reg::RD_MAC_SPI_SYS_3).map(|v| (v >> 18) & 7), Some(3));
        // Calibration fields, read the way esp-hal's read_field_le does.
        let field = |block: u32, start: u32, width: u32| {
            let base = if block == 1 {
                reg::RD_MAC_SPI_SYS_0
            } else {
                reg::RD_SYS_PART1_DATA0
            };
            (0..width).fold(0, |acc, i| {
                let bit = start + i;
                acc | ((e.read(base + 4 * (bit / 32)).unwrap() >> (bit % 32)) & 1) << i
            })
        };
        assert_eq!(field(2, 128, 2), 1, "BLK_VERSION_MAJOR");
        assert_eq!(field(2, 148, 10) + 1000, 1390, "ADC1 init code, atten 0");
        assert_eq!(field(2, 158, 10), 0b10_0001_1011, "word-straddling field");
        assert_eq!(field(2, 218, 10), 0b10_0100_0111, "ADC1_CAL_VOL_ATTEN3");
        assert_eq!(field(1, 170, 10), 500, "THRES_HVT");
        assert_eq!(
            e.read(reg::RD_MAC_SPI_SYS_3).map(|v| (v >> 18) & 7),
            Some(3),
            "wafer version kept"
        );
        assert_eq!([0, 1, 2, 3].map(adc1_cal_code), [1947, 2013, 1962, 1929]);
        assert_eq!(e.read(0x30), Some(0), "no secure boot, no encryption");
        e.write(reg::CMD, 1).unwrap();
        assert_eq!(e.read(reg::CMD), Some(0), "read command done");
        e.write(reg::RD_MAC_SPI_SYS_3, 0).unwrap();
        assert_eq!(e.read(reg::RD_MAC_SPI_SYS_3), Some(3 << 18), "read-out is read-only");
        assert_eq!(e.read(reg::STATUS), Some(1));
    }
}
