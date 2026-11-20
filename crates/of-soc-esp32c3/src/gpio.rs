//! GPIO, GPIO matrix output selection and IO MUX (TRM chapter 5).
//!
//! Modelled: `GPIO_OUT`, `GPIO_ENABLE` and their W1TS/W1TC aliases,
//! `GPIO_IN` (pin inputs set by the SoC), `GPIO_FUNCn_OUT_SEL_CFG`
//! (`OUT_SEL` 128 = "GPIO_OUT bit n" and `INV_SEL`), and the IO MUX
//! `MCU_SEL` and `FUN_DRV` fields, `GPIO_PINn` (`PAD_DRIVER`: open drain)
//! and `GPIO_FUNCn_IN_SEL_CFG` (input routing, stored only). A pin drives
//! only when its IO MUX function is 1 (GPIO matrix), its `OUT_SEL` is 128
//! and its `GPIO_ENABLE` bit is set; other selections leave it
//! high-impedance for now. An open-drain pin drives low and releases
//! (high-impedance) for high. The other `GPIO_PINn` fields (pin
//! interrupts, wake-up) are stored without effect.

/// Number of GPIO pins on the ESP32-C3 (GPIO0 to GPIO21).
pub const GPIO_COUNT: usize = 22;
const PIN_MASK: u32 = (1 << GPIO_COUNT) - 1;

/// Register offsets from [`crate::map::GPIO_BASE`].
pub mod reg {
    /// Output levels.
    pub const OUT: u32 = 0x04;
    /// Set output bits.
    pub const OUT_W1TS: u32 = 0x08;
    /// Clear output bits.
    pub const OUT_W1TC: u32 = 0x0c;
    /// Output enables.
    pub const ENABLE: u32 = 0x20;
    /// Set enable bits.
    pub const ENABLE_W1TS: u32 = 0x24;
    /// Clear enable bits.
    pub const ENABLE_W1TC: u32 = 0x28;
    /// Input levels (read-only).
    pub const IN: u32 = 0x3c;
    /// `GPIO_PIN0_REG`; pin n is at `+ 4 * n`.
    pub const PIN: u32 = 0x74;
    /// `GPIO_FUNC0_IN_SEL_CFG`; peripheral input signal n is at `+ 4 * n`
    /// (128 signals).
    pub const FUNC_IN_SEL: u32 = 0x154;
    /// `GPIO_FUNC0_OUT_SEL_CFG`; pin n is at `+ 4 * n`.
    pub const FUNC_OUT_SEL: u32 = 0x554;
}

/// `OUT_SEL` value meaning "drive from `GPIO_OUT` bit n".
pub const SIG_GPIO_OUT: u32 = 128;

/// What a pin's output stage is doing.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PinOut {
    /// `Some(level)` when driving, `None` when high-impedance.
    pub level: Option<bool>,
    /// IO MUX drive strength `FUN_DRV` (0 to 3: about 5, 10, 20, 40 mA).
    pub strength: u8,
}

impl PinOut {
    /// Not driving.
    pub const HIZ: Self = Self {
        level: None,
        strength: 2,
    };

    /// Thevenin model for an analog pin driver: `(volts, ohms)`. The output
    /// resistance per drive strength is an estimate (about 120, 60, 30 and
    /// 15 ohms), to be calibrated on hardware; high-impedance is 1 GΩ.
    pub fn thevenin(self, vdd: f64) -> (f64, f64) {
        match self.level {
            None => (0.0, 1e9),
            Some(l) => (
                if l { vdd } else { 0.0 },
                120.0 / f64::from(1u32 << self.strength.min(3)),
            ),
        }
    }
}

/// GPIO and IO MUX register state.
#[derive(Debug, Clone)]
pub struct Gpio {
    out: u32,
    enable: u32,
    input: u32,
    out_sel: [u32; GPIO_COUNT],
    io_mux: [u32; GPIO_COUNT],
    pin_cfg: [u32; GPIO_COUNT],
    in_sel: [u32; IN_SIGNALS],
}

/// Peripheral input signals routed by `GPIO_FUNCn_IN_SEL_CFG`.
const IN_SIGNALS: usize = 128;
/// `GPIO_PINn.PAD_DRIVER`: open drain.
const PAD_DRIVER: u32 = 1 << 2;

impl Default for Gpio {
    fn default() -> Self {
        Self::new()
    }
}

const MCU_SEL_SHIFT: u32 = 12;
const FUN_DRV_SHIFT: u32 = 10;

impl Gpio {
    /// Reset state: outputs off, `OUT_SEL` 128 on every pin; IO MUX
    /// function 1 (GPIO) and drive 2 except the flash pins (12 to 17) and
    /// UART0 (20, 21), which start on function 0.
    pub fn new() -> Self {
        let mut io_mux = [0; GPIO_COUNT];
        for (pin, m) in io_mux.iter_mut().enumerate() {
            let func = if (12..=17).contains(&pin) || pin >= 20 { 0 } else { 1 };
            *m = (func << MCU_SEL_SHIFT) | (2 << FUN_DRV_SHIFT);
        }
        Self {
            out: 0,
            enable: 0,
            input: 0,
            out_sel: [SIG_GPIO_OUT | (1 << 10); GPIO_COUNT],
            io_mux,
            pin_cfg: [0; GPIO_COUNT],
            in_sel: [0; IN_SIGNALS],
        }
    }

    /// Read a GPIO register at offset `off`.
    pub fn read(&self, off: u32) -> Option<u32> {
        Some(match off {
            reg::OUT => self.out,
            reg::ENABLE => self.enable,
            reg::IN => self.input,
            reg::OUT_W1TS | reg::OUT_W1TC | reg::ENABLE_W1TS | reg::ENABLE_W1TC => 0,
            o if (reg::FUNC_OUT_SEL..reg::FUNC_OUT_SEL + 4 * GPIO_COUNT as u32).contains(&o) => {
                self.out_sel[((o - reg::FUNC_OUT_SEL) / 4) as usize]
            }
            o if (reg::PIN..reg::PIN + 4 * GPIO_COUNT as u32).contains(&o) => {
                self.pin_cfg[((o - reg::PIN) / 4) as usize]
            }
            o if (reg::FUNC_IN_SEL..reg::FUNC_IN_SEL + 4 * IN_SIGNALS as u32).contains(&o) => {
                self.in_sel[((o - reg::FUNC_IN_SEL) / 4) as usize]
            }
            _ => return None,
        })
    }

    /// Write a GPIO register. `None` if the offset is not modelled.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        match off {
            reg::OUT => self.out = v & PIN_MASK,
            reg::OUT_W1TS => self.out |= v & PIN_MASK,
            reg::OUT_W1TC => self.out &= !v,
            reg::ENABLE => self.enable = v & PIN_MASK,
            reg::ENABLE_W1TS => self.enable |= v & PIN_MASK,
            reg::ENABLE_W1TC => self.enable &= !v,
            reg::IN => {}
            o if (reg::FUNC_OUT_SEL..reg::FUNC_OUT_SEL + 4 * GPIO_COUNT as u32).contains(&o) => {
                self.out_sel[((o - reg::FUNC_OUT_SEL) / 4) as usize] = v & 0x7ff;
            }
            o if (reg::PIN..reg::PIN + 4 * GPIO_COUNT as u32).contains(&o) => {
                self.pin_cfg[((o - reg::PIN) / 4) as usize] = v & 0x3_ff9f;
            }
            o if (reg::FUNC_IN_SEL..reg::FUNC_IN_SEL + 4 * IN_SIGNALS as u32).contains(&o) => {
                self.in_sel[((o - reg::FUNC_IN_SEL) / 4) as usize] = v & 0x7f;
            }
            _ => return None,
        }
        Some(())
    }

    /// Read an IO MUX register (`IO_MUX_GPIOn_REG` at `4 + 4 * n`).
    pub fn io_mux_read(&self, off: u32) -> Option<u32> {
        let pin = (off.checked_sub(4)? / 4) as usize;
        off.is_multiple_of(4).then_some(())?;
        self.io_mux.get(pin).copied()
    }

    /// Write an IO MUX register.
    pub fn io_mux_write(&mut self, off: u32, v: u32) -> Option<()> {
        let pin = (off.checked_sub(4)? / 4) as usize;
        off.is_multiple_of(4).then_some(())?;
        *self.io_mux.get_mut(pin)? = v;
        Some(())
    }

    /// Set the level seen on `GPIO_IN` for `pin`.
    pub fn set_input(&mut self, pin: usize, level: bool) {
        if pin < GPIO_COUNT {
            self.input = (self.input & !(1 << pin)) | (u32::from(level) << pin);
        }
    }

    /// What pin `pin`'s output stage is doing.
    pub fn pin_out(&self, pin: usize) -> PinOut {
        let mux = self.io_mux[pin];
        let strength = ((mux >> FUN_DRV_SHIFT) & 3) as u8;
        let sel = self.out_sel[pin];
        let gpio_fn = (mux >> MCU_SEL_SHIFT) & 7 == 1;
        if !gpio_fn || sel & 0xff != SIG_GPIO_OUT || self.enable & (1 << pin) == 0 {
            return PinOut { level: None, strength };
        }
        let inv = sel & (1 << 8) != 0;
        let high = (self.out >> pin & 1 != 0) != inv;
        let open_drain = self.pin_cfg[pin] & PAD_DRIVER != 0;
        PinOut {
            level: if high && open_drain { None } else { Some(high) },
            strength,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn w1ts_w1tc_and_enable_drive_the_pin() {
        let mut g = Gpio::new();
        assert_eq!(g.pin_out(2), PinOut::HIZ);
        g.write(reg::OUT_W1TS, 1 << 2).unwrap();
        assert_eq!(g.pin_out(2).level, None, "not enabled yet");
        g.write(reg::ENABLE_W1TS, 1 << 2).unwrap();
        assert_eq!(g.pin_out(2).level, Some(true));
        g.write(reg::OUT_W1TC, 1 << 2).unwrap();
        assert_eq!(g.pin_out(2).level, Some(false));
        assert_eq!(g.read(reg::OUT), Some(0));
        assert_eq!(g.read(reg::ENABLE), Some(1 << 2));
        g.write(reg::ENABLE_W1TC, 1 << 2).unwrap();
        assert_eq!(g.pin_out(2).level, None);
        g.write(reg::OUT, u32::MAX).unwrap();
        assert_eq!(g.read(reg::OUT), Some(PIN_MASK), "only 22 pins");
    }

    #[test]
    fn out_sel_inversion_and_io_mux_function() {
        let mut g = Gpio::new();
        g.write(reg::ENABLE_W1TS, 1 << 3).unwrap();
        g.write(reg::FUNC_OUT_SEL + 12, SIG_GPIO_OUT | (1 << 8)).unwrap();
        assert_eq!(g.pin_out(3).level, Some(true), "inverted low");
        g.write(reg::FUNC_OUT_SEL + 12, 6).unwrap(); // a peripheral signal
        assert_eq!(g.pin_out(3).level, None);
        g.write(reg::FUNC_OUT_SEL + 12, SIG_GPIO_OUT).unwrap();
        g.io_mux_write(4 + 12, 3 << 10).unwrap(); // function 0, drive 3
        assert_eq!(
            g.pin_out(3),
            PinOut {
                level: None,
                strength: 3
            }
        );
        g.io_mux_write(4 + 12, (1 << 12) | (3 << 10)).unwrap();
        assert_eq!(
            g.pin_out(3),
            PinOut {
                level: Some(false),
                strength: 3
            }
        );
        // UART0 TX (GPIO21) starts on function 0.
        g.write(reg::ENABLE_W1TS, 1 << 21).unwrap();
        assert_eq!(g.pin_out(21).level, None);
    }

    #[test]
    fn open_drain_releases_high_and_in_sel_is_stored() {
        let mut g = Gpio::new();
        g.write(reg::ENABLE_W1TS, 1 << 8).unwrap();
        g.write(reg::PIN + 4 * 8, PAD_DRIVER).unwrap();
        assert_eq!(g.read(reg::PIN + 4 * 8), Some(PAD_DRIVER));
        assert_eq!(g.pin_out(8).level, Some(false), "drives low");
        g.write(reg::OUT_W1TS, 1 << 8).unwrap();
        assert_eq!(g.pin_out(8).level, None, "releases high");
        g.write(reg::PIN + 4 * 8, 0).unwrap();
        assert_eq!(g.pin_out(8).level, Some(true), "push-pull again");
        // U0RXD_IN (signal 6) from GPIO20.
        g.write(reg::FUNC_IN_SEL + 4 * 6, 20 | 1 << 6).unwrap();
        assert_eq!(g.read(reg::FUNC_IN_SEL + 4 * 6), Some(20 | 1 << 6));
        assert_eq!(g.read(reg::FUNC_IN_SEL + 4 * 128), None);
    }

    #[test]
    fn input_register_is_read_only() {
        let mut g = Gpio::new();
        g.set_input(5, true);
        g.write(reg::IN, 0).unwrap();
        assert_eq!(g.read(reg::IN), Some(1 << 5));
    }

    #[test]
    fn thevenin_per_strength() {
        let p = PinOut {
            level: Some(true),
            strength: 2,
        };
        assert_eq!(p.thevenin(3.3), (3.3, 30.0));
        assert_eq!(
            PinOut {
                level: Some(false),
                strength: 0
            }
            .thevenin(3.3),
            (0.0, 120.0)
        );
        assert_eq!(PinOut::HIZ.thevenin(3.3).1, 1e9);
    }
}
