//! GPIO, GPIO matrix and IO MUX (TRM chapter 5).
//!
//! Outputs: `GPIO_OUT`, `GPIO_ENABLE` and their W1TS/W1TC aliases,
//! `GPIO_FUNCn_OUT_SEL_CFG` (`OUT_SEL` 128 = "GPIO_OUT bit n" and
//! `INV_SEL`), the IO MUX `MCU_SEL` and `FUN_DRV` fields and `GPIO_PINn`
//! `PAD_DRIVER` (open drain). A pin drives only when its IO MUX function is
//! 1 (GPIO matrix) and its output is enabled: with `OUT_SEL` 128 it drives
//! `GPIO_OUT` when its `GPIO_ENABLE` bit is set; with `OUT_SEL` naming a
//! modelled peripheral output signal (the LEDC's `LEDC_LS_SIG_OUT0..5`,
//! 45 to 50, see [`Gpio::set_output_signal`]) it drives that signal, enabled
//! by `GPIO_ENABLE` when `OEN_SEL` is set and otherwise by the
//! peripheral's own enable (always 1 for the LEDC, TRM table 5.11-1),
//! inverted by `OEN_INV_SEL`. `INV_SEL` inverts the level. Other signals
//! leave the pin high-impedance. An open-drain pin drives low and releases
//! (high-impedance) for high.
//!
//! Pads: the IO MUX `FUN_WPU`/`FUN_WPD` weak pull-up and pull-down (about
//! 45 kΩ, the datasheet's typical value) are part of the pin's analog
//! model ([`PinOut::thevenin`]), and `FUN_IE` gates the input. Reset values
//! follow the datasheet's pin table ("after reset" column) and default
//! drive strengths.
//!
//! Inputs: `GPIO_IN` reads the pad levels of pins with `FUN_IE` set. A pad
//! level comes from outside ([`Gpio::set_input`], for example the analog
//! solver through the engine) for pins marked external with
//! [`Gpio::set_external`]; otherwise the pad follows the pin itself (its
//! driven level, else its pull, else the last level set). The input
//! synchronisers and glitch filter add no delay here.
//!
//! Interrupts: `GPIO_PINn_INT_TYPE` (rising, falling, any edge, low or high
//! level) sets `GPIO_STATUS` bits (with W1TS/W1TC); level types set them
//! again while the level lasts. `INT_ENA` bit 0 routes a pin to the CPU
//! interrupt (`GPIO_PCPU_INT`, interrupt matrix source 16) and bit 1 to the
//! non-maskable one (`GPIO_PCPU_NMI_INT`, source 17).
//!
//! `GPIO_FUNCn_IN_SEL_CFG` routes pins to peripheral input signals
//! ([`Gpio::peripheral_input`]); no modelled peripheral reads them yet.

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
    /// Interrupt status.
    pub const STATUS: u32 = 0x44;
    /// Set interrupt status bits.
    pub const STATUS_W1TS: u32 = 0x48;
    /// Clear interrupt status bits.
    pub const STATUS_W1TC: u32 = 0x4c;
    /// Status of pins routed to the CPU interrupt (read-only).
    pub const PCPU_INT: u32 = 0x5c;
    /// Status of pins routed to the CPU non-maskable interrupt (read-only).
    pub const PCPU_NMI_INT: u32 = 0x60;
    /// SDIO interrupt status (read-only, always 0 here).
    pub const CPUSDIO_INT: u32 = 0x64;
    /// `GPIO_PIN0_REG`; pin n is at `+ 4 * n`.
    pub const PIN: u32 = 0x74;
    /// `GPIO_FUNC0_IN_SEL_CFG`; peripheral input signal n is at `+ 4 * n`
    /// (128 signals).
    pub const FUNC_IN_SEL: u32 = 0x154;
    /// `GPIO_FUNC0_OUT_SEL_CFG`; pin n is at `+ 4 * n`.
    pub const FUNC_OUT_SEL: u32 = 0x554;
}

/// IO MUX register fields (`IO_MUX_GPIOn_REG`).
pub mod io_mux {
    /// Pull-down enable.
    pub const FUN_WPD: u32 = 1 << 7;
    /// Pull-up enable.
    pub const FUN_WPU: u32 = 1 << 8;
    /// Input enable.
    pub const FUN_IE: u32 = 1 << 9;
    /// Drive strength (2 bits).
    pub const FUN_DRV_SHIFT: u32 = 10;
    /// Function select (3 bits); 1 is the GPIO matrix.
    pub const MCU_SEL_SHIFT: u32 = 12;
}

/// `GPIO_PINn` fields.
pub mod pin_reg {
    /// Open drain (`PAD_DRIVER`).
    pub const PAD_DRIVER: u32 = 1 << 2;
    /// Interrupt type (3 bits): 0 off, 1 rising, 2 falling, 3 any edge,
    /// 4 low level, 5 high level.
    pub const INT_TYPE_SHIFT: u32 = 7;
    /// Interrupt enable (5 bits): bit 0 CPU interrupt, bit 1 CPU NMI.
    pub const INT_ENA_SHIFT: u32 = 13;
}

/// `OUT_SEL` value meaning "drive from `GPIO_OUT` bit n".
pub const SIG_GPIO_OUT: u32 = 128;
/// `IN_SEL` value for a constant 1 input.
pub const IN_CONST_ONE: u32 = 0x1e;
/// `IN_SEL` value for a constant 0 input.
pub const IN_CONST_ZERO: u32 = 0x1f;

/// Weak pull-up and pull-down resistance (Ω): the datasheet's typical
/// `R_PU`/`R_PD`.
pub const R_PULL: f64 = 45_000.0;
/// Input high threshold as a fraction of VDD (datasheet `V_IH` minimum).
pub const VIH_FRACTION: f64 = 0.75;
/// Input low threshold as a fraction of VDD (datasheet `V_IL` maximum).
pub const VIL_FRACTION: f64 = 0.25;

/// What a pin's pad is doing towards the outside: its output stage and
/// its weak pulls.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct PinOut {
    /// `Some(level)` when driving, `None` when high-impedance.
    pub level: Option<bool>,
    /// IO MUX drive strength `FUN_DRV` (0 to 3: about 5, 10, 20, 40 mA).
    pub strength: u8,
    /// Weak pull-up on (`FUN_WPU`).
    pub pull_up: bool,
    /// Weak pull-down on (`FUN_WPD`).
    pub pull_down: bool,
}

impl PinOut {
    /// Not driving, no pulls.
    pub const HIZ: Self = Self {
        level: None,
        strength: 2,
        pull_up: false,
        pull_down: false,
    };

    /// Driving `level` at drive strength `strength`, no pulls.
    pub const fn driven(level: bool, strength: u8) -> Self {
        Self {
            level: Some(level),
            strength,
            pull_up: false,
            pull_down: false,
        }
    }

    /// Thevenin model for an analog pin driver: `(volts, ohms)`. The output
    /// resistance per drive strength is an estimate (about 120, 60, 30 and
    /// 15 ohms), to be calibrated on hardware; the weak pulls are [`R_PULL`]
    /// to VDD or ground in parallel; a pad doing nothing is 1 GΩ to ground.
    pub fn thevenin(self, vdd: f64) -> (f64, f64) {
        let driver = self.level.map(|l| {
            (
                if l { vdd } else { 0.0 },
                120.0 / f64::from(1u32 << self.strength.min(3)),
            )
        });
        let sources = [
            driver,
            self.pull_up.then_some((vdd, R_PULL)),
            self.pull_down.then_some((0.0, R_PULL)),
        ];
        match sources.iter().flatten().count() {
            0 => (0.0, 1e9),
            1 => sources.into_iter().flatten().next().unwrap_or((0.0, 1e9)),
            _ => {
                // Sources in parallel: sum the conductances and currents.
                let (g, i) = sources
                    .into_iter()
                    .flatten()
                    .fold((0.0, 0.0), |(g, i), (v, r)| (g + 1.0 / r, i + v / r));
                (i / g, 1.0 / g)
            }
        }
    }
}

/// GPIO and IO MUX register state.
#[derive(Debug, Clone)]
pub struct Gpio {
    out: u32,
    enable: u32,
    out_sel: [u32; GPIO_COUNT],
    io_mux: [u32; GPIO_COUNT],
    pin_cfg: [u32; GPIO_COUNT],
    in_sel: [u32; IN_SIGNALS],
    /// Pad levels set from outside (or last set) per pin.
    pad: u32,
    /// Pins whose pad level comes from outside.
    external: u32,
    /// `GPIO_IN` as last evaluated (pad levels gated by `FUN_IE`).
    input: u32,
    status: u32,
    /// Levels of the peripheral output signals (bit = signal number).
    signals: u128,
}

/// `U0TXD_OUT`, UART0's TX output signal in the GPIO matrix (TRM table
/// 5.11-1). GPIO21's IO MUX function 0 is U0TXD directly (ESP32-C3 Series
/// Datasheet v2.5, Table 2-4 IO MUX Pin Functions).
pub const U0TXD_OUT: usize = 6;
/// The pin whose IO MUX function 0 is U0TXD.
pub const U0TXD_PIN: usize = 21;
/// Peripheral output signals a modelled peripheral drives: UART0 TX (6)
/// and the LEDC channels (45 to 50).
const DRIVEN_SIGNALS: u128 = (0x3f << 45) | (1 << U0TXD_OUT);
/// `GPIO_FUNCn_OUT_SEL_CFG` bits.
const INV_SEL: u32 = 1 << 8;
const OEN_SEL: u32 = 1 << 9;
const OEN_INV_SEL: u32 = 1 << 10;

/// Peripheral input signals routed by `GPIO_FUNCn_IN_SEL_CFG`.
const IN_SIGNALS: usize = 128;
/// Pins with the input enabled after reset (datasheet pin table).
const RESET_IE: u32 = 0b0001_0011_1111_0111_1111_1100;
/// Pins with the weak pull-up on after reset: MTCK (GPIO6), GPIO9, the
/// SPI flash pins (12 to 17) and UART0 (20, 21).
const RESET_WPU: u32 = (1 << 6) | (1 << 9) | (0x3f << 12) | (1 << 20) | (1 << 21);

impl Default for Gpio {
    fn default() -> Self {
        Self::new()
    }
}

impl Gpio {
    /// Reset state: outputs off, `OUT_SEL` 128 on every pin; IO MUX
    /// function 1 (GPIO) except the flash pins (12 to 17) and UART0 (20,
    /// 21), which start on function 0; drive strength 1 (10 mA) on GPIO2
    /// to GPIO5, 3 (40 mA) on GPIO18 and GPIO19 and 2 (20 mA) elsewhere;
    /// input enable and pull-ups as the datasheet's pin table gives them.
    pub fn new() -> Self {
        let io_mux = std::array::from_fn(|pin| {
            let func = if (12..=17).contains(&pin) || pin >= 20 { 0 } else { 1 };
            let drv = match pin {
                2..=5 => 1,
                18 | 19 => 3,
                _ => 2,
            };
            let bit = |mask: u32, f: u32| if mask >> pin & 1 != 0 { f } else { 0 };
            (func << io_mux::MCU_SEL_SHIFT)
                | (drv << io_mux::FUN_DRV_SHIFT)
                | bit(RESET_IE, io_mux::FUN_IE)
                | bit(RESET_WPU, io_mux::FUN_WPU)
        });
        let mut g = Self {
            out: 0,
            enable: 0,
            out_sel: [SIG_GPIO_OUT | (1 << 10); GPIO_COUNT],
            io_mux,
            pin_cfg: [0; GPIO_COUNT],
            in_sel: [0; IN_SIGNALS],
            pad: 0,
            external: 0,
            input: 0,
            status: 0,
            // The UART TX line idles high.
            signals: 1 << U0TXD_OUT,
        };
        g.input = g.input_levels();
        g
    }

    /// Reset the registers (a system or core reset). Pad levels set from
    /// outside and the external pin set stay: they belong to the circuit.
    pub fn reset(&mut self) {
        let (pad, external) = (self.pad, self.external);
        *self = Self::new();
        self.pad = pad;
        self.external = external;
        self.input = self.input_levels();
    }

    fn pin_index(off: u32, base: u32, count: usize) -> Option<usize> {
        (off >= base && off < base + 4 * count as u32 && off.is_multiple_of(4)).then(|| ((off - base) / 4) as usize)
    }

    /// Read a GPIO register at offset `off`.
    pub fn read(&self, off: u32) -> Option<u32> {
        if let Some(n) = Self::pin_index(off, reg::FUNC_OUT_SEL, GPIO_COUNT) {
            return Some(self.out_sel[n]);
        }
        if let Some(n) = Self::pin_index(off, reg::PIN, GPIO_COUNT) {
            return Some(self.pin_cfg[n]);
        }
        if let Some(n) = Self::pin_index(off, reg::FUNC_IN_SEL, IN_SIGNALS) {
            return Some(self.in_sel[n]);
        }
        Some(match off {
            reg::OUT => self.out,
            reg::ENABLE => self.enable,
            reg::IN => self.input,
            reg::STATUS => self.status,
            reg::PCPU_INT => self.status & self.int_ena_mask(0),
            reg::PCPU_NMI_INT => self.status & self.int_ena_mask(1),
            reg::CPUSDIO_INT => 0,
            reg::OUT_W1TS | reg::OUT_W1TC | reg::ENABLE_W1TS | reg::ENABLE_W1TC => 0,
            reg::STATUS_W1TS | reg::STATUS_W1TC => 0,
            _ => return None,
        })
    }

    /// Write a GPIO register. `None` if the offset is not modelled.
    pub fn write(&mut self, off: u32, v: u32) -> Option<()> {
        if let Some(n) = Self::pin_index(off, reg::FUNC_OUT_SEL, GPIO_COUNT) {
            self.out_sel[n] = v & 0x7ff;
        } else if let Some(n) = Self::pin_index(off, reg::PIN, GPIO_COUNT) {
            self.pin_cfg[n] = v & 0x3_ff9f;
        } else if let Some(n) = Self::pin_index(off, reg::FUNC_IN_SEL, IN_SIGNALS) {
            self.in_sel[n] = v & 0x7f;
        } else {
            match off {
                reg::OUT => self.out = v & PIN_MASK,
                reg::OUT_W1TS => self.out |= v & PIN_MASK,
                reg::OUT_W1TC => self.out &= !v,
                reg::ENABLE => self.enable = v & PIN_MASK,
                reg::ENABLE_W1TS => self.enable |= v & PIN_MASK,
                reg::ENABLE_W1TC => self.enable &= !v,
                reg::STATUS => self.status = v & PIN_MASK,
                reg::STATUS_W1TS => self.status |= v & PIN_MASK,
                reg::STATUS_W1TC => self.status &= !v,
                reg::IN | reg::PCPU_INT | reg::PCPU_NMI_INT | reg::CPUSDIO_INT => {}
                _ => return None,
            }
        }
        self.update_inputs();
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
        self.update_inputs();
        Some(())
    }

    /// Set the pad level of `pin` as seen from outside (an external
    /// driver, or the analog node's level). Edges raise pin interrupts.
    pub fn set_input(&mut self, pin: usize, level: bool) {
        if pin < GPIO_COUNT {
            self.pad = (self.pad & !(1 << pin)) | (u32::from(level) << pin);
            self.update_inputs();
        }
    }

    /// Whether `pin`'s pad level comes from outside ([`Self::set_input`])
    /// rather than from the pin's own output and pulls.
    /// A pin that becomes external keeps its current pad level until
    /// [`Self::set_input`] says otherwise, so no edge is raised.
    pub fn set_external(&mut self, pin: usize, external: bool) {
        if pin < GPIO_COUNT {
            let level = self.pad_levels() & (1 << pin);
            self.pad = (self.pad & !(1 << pin)) | level;
            self.external = (self.external & !(1 << pin)) | (u32::from(external) << pin);
            self.update_inputs();
        }
    }

    /// The pad level of every pin (bit n = GPIOn).
    pub fn pad_levels(&self) -> u32 {
        let mut levels = self.pad & self.external;
        for pin in (0..GPIO_COUNT).filter(|&p| self.external >> p & 1 == 0) {
            let o = self.pin_out(pin);
            let level = match (o.level, o.pull_up, o.pull_down) {
                (Some(l), _, _) => l,
                (None, true, false) => true,
                (None, false, true) => false,
                _ => self.pad >> pin & 1 != 0,
            };
            levels |= u32::from(level) << pin;
        }
        levels
    }

    fn input_levels(&self) -> u32 {
        let ie = (0..GPIO_COUNT).fold(0, |m, p| m | u32::from(self.io_mux[p] & io_mux::FUN_IE != 0) << p);
        self.pad_levels() & ie
    }

    fn int_ena_mask(&self, bit: u32) -> u32 {
        (0..GPIO_COUNT).fold(0, |m, p| {
            m | ((self.pin_cfg[p] >> (pin_reg::INT_ENA_SHIFT + bit)) & 1) << p
        })
    }

    /// Re-evaluate `GPIO_IN` and raise pin interrupts for its changes and
    /// for held levels.
    fn update_inputs(&mut self) {
        let new = self.input_levels();
        let changed = new ^ self.input;
        for p in 0..GPIO_COUNT {
            let bit = 1 << p;
            let hit = match (self.pin_cfg[p] >> pin_reg::INT_TYPE_SHIFT) & 7 {
                1 => changed & new & bit,
                2 => changed & !new & bit,
                3 => changed & bit,
                4 => !new & bit,
                5 => new & bit,
                _ => 0,
            };
            self.status |= hit;
        }
        self.input = new;
    }

    /// Interrupt outputs: (CPU interrupt, CPU NMI), the GPIO and GPIO_NMI
    /// interrupt matrix sources.
    pub fn irq(&self) -> (bool, bool) {
        (
            self.status & self.int_ena_mask(0) != 0,
            self.status & self.int_ena_mask(1) != 0,
        )
    }

    /// The level peripheral input `signal` sees through the GPIO matrix
    /// (`GPIO_FUNCn_IN_SEL_CFG`: a pin's `GPIO_IN` level or a constant,
    /// optionally inverted). `None` when the signal bypasses the matrix
    /// (`SEL` = 0, IO MUX direct input, not modelled) or does not exist.
    pub fn peripheral_input(&self, signal: usize) -> Option<bool> {
        let cfg = *self.in_sel.get(signal)?;
        if cfg & (1 << 6) == 0 {
            return None;
        }
        let level = match cfg & 0x1f {
            IN_CONST_ONE => true,
            IN_CONST_ZERO => false,
            p if (p as usize) < GPIO_COUNT => self.input >> p & 1 != 0,
            _ => false,
        };
        Some(level != (cfg & (1 << 5) != 0))
    }

    /// Set peripheral output signal `signal` (the GPIO matrix's numbering)
    /// to `level`. Pins routed to it follow, and so do their input
    /// loopbacks and pin interrupts.
    pub fn set_output_signal(&mut self, signal: usize, level: bool) {
        if signal < 128 {
            self.signals = (self.signals & !(1 << signal)) | (u128::from(level) << signal);
            self.update_inputs();
        }
    }

    /// What pin `pin`'s pad is doing: output stage and pulls.
    pub fn pin_out(&self, pin: usize) -> PinOut {
        let mux = self.io_mux[pin];
        let strength = ((mux >> io_mux::FUN_DRV_SHIFT) & 3) as u8;
        let pull_up = mux & io_mux::FUN_WPU != 0;
        let pull_down = mux & io_mux::FUN_WPD != 0;
        let hiz = PinOut {
            level: None,
            strength,
            pull_up,
            pull_down,
        };
        if pin == U0TXD_PIN && (mux >> io_mux::MCU_SEL_SHIFT) & 7 == 0 {
            // IO MUX function 0: U0TXD, driven by UART0 (output always enabled).
            return PinOut {
                level: Some(self.signals >> U0TXD_OUT & 1 != 0),
                ..hiz
            };
        }
        let sel = self.out_sel[pin];
        let gpio_fn = (mux >> io_mux::MCU_SEL_SHIFT) & 7 == 1;
        let sig = sel & 0xff;
        let enabled = self.enable & (1 << pin) != 0;
        let level = if sig == SIG_GPIO_OUT {
            enabled.then_some(self.out >> pin & 1 != 0)
        } else if DRIVEN_SIGNALS >> sig & 1 != 0 {
            let oe = if sel & OEN_SEL != 0 {
                enabled
            } else {
                sel & OEN_INV_SEL == 0
            };
            oe.then_some(self.signals >> sig & 1 != 0)
        } else {
            None
        };
        let Some(level) = level.filter(|_| gpio_fn) else {
            return hiz;
        };
        let high = level != (sel & INV_SEL != 0);
        let open_drain = self.pin_cfg[pin] & pin_reg::PAD_DRIVER != 0;
        PinOut {
            level: if high && open_drain { None } else { Some(high) },
            ..hiz
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const fn pin_cfg(int_type: u32, int_ena: u32) -> u32 {
        (int_type << pin_reg::INT_TYPE_SHIFT) | (int_ena << pin_reg::INT_ENA_SHIFT)
    }

    #[test]
    fn w1ts_w1tc_and_enable_drive_the_pin() {
        let mut g = Gpio::new();
        assert_eq!(
            g.pin_out(2),
            PinOut {
                strength: 1,
                ..PinOut::HIZ
            }
        );
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
    fn gpio21_is_u0txd_after_reset_and_through_the_matrix() {
        let mut g = Gpio::new();
        assert_eq!(g.pin_out(U0TXD_PIN).level, Some(true), "idle high from reset");
        g.set_output_signal(U0TXD_OUT, false);
        assert_eq!(g.pin_out(U0TXD_PIN).level, Some(false));
        // GPIO3 through the matrix: OUT_SEL = U0TXD_OUT, function 1.
        g.write(reg::FUNC_OUT_SEL + 12, U0TXD_OUT as u32).unwrap();
        assert_eq!(g.pin_out(3).level, Some(false));
        g.set_output_signal(U0TXD_OUT, true);
        assert_eq!(g.pin_out(3).level, Some(true));
        g.reset();
        assert_eq!(g.pin_out(U0TXD_PIN).level, Some(true));
    }

    #[test]
    fn peripheral_signals_drive_routed_pins() {
        let mut g = Gpio::new();
        // esp-hal's LEDC hookup: OUT_SEL 45 (LEDC channel 0), OEN_SEL so
        // GPIO_ENABLE decides, output enabled.
        g.write(reg::FUNC_OUT_SEL + 4 * 8, 45 | OEN_SEL).unwrap();
        g.write(reg::OUT_W1TS, 1 << 8).unwrap();
        assert_eq!(g.pin_out(8).level, None, "GPIO_ENABLE clear");
        g.write(reg::ENABLE_W1TS, 1 << 8).unwrap();
        assert_eq!(g.pin_out(8).level, Some(false), "follows the signal, not GPIO_OUT");
        g.set_output_signal(45, true);
        assert_eq!(g.pin_out(8).level, Some(true));
        assert_eq!(g.read(reg::IN).map(|v| v >> 8 & 1), Some(1), "input loopback");
        g.write(reg::FUNC_OUT_SEL + 4 * 8, 45 | OEN_SEL | INV_SEL).unwrap();
        assert_eq!(g.pin_out(8).level, Some(false), "INV_SEL");
        // The LEDC's own output enable is 1, unless inverted.
        g.write(reg::ENABLE_W1TC, 1 << 8).unwrap();
        g.write(reg::FUNC_OUT_SEL + 4 * 8, 45).unwrap();
        assert_eq!(g.pin_out(8).level, Some(true));
        g.write(reg::FUNC_OUT_SEL + 4 * 8, 45 | OEN_INV_SEL).unwrap();
        assert_eq!(g.pin_out(8).level, None);
        // Signals no modelled peripheral drives stay high-impedance.
        g.set_output_signal(51, true);
        g.write(reg::FUNC_OUT_SEL + 4 * 8, 51).unwrap();
        assert_eq!(g.pin_out(8).level, None);
    }

    #[test]
    fn out_sel_inversion_and_io_mux_function() {
        let mut g = Gpio::new();
        g.write(reg::ENABLE_W1TS, 1 << 3).unwrap();
        g.write(reg::FUNC_OUT_SEL + 12, SIG_GPIO_OUT | (1 << 8)).unwrap();
        assert_eq!(g.pin_out(3).level, Some(true), "inverted low");
        g.write(reg::FUNC_OUT_SEL + 12, 9).unwrap(); // an unmodelled peripheral signal
        assert_eq!(g.pin_out(3).level, None);
        g.write(reg::FUNC_OUT_SEL + 12, SIG_GPIO_OUT).unwrap();
        g.io_mux_write(4 + 12, 3 << 10).unwrap(); // function 0, drive 3
        assert_eq!(
            g.pin_out(3),
            PinOut {
                strength: 3,
                ..PinOut::HIZ
            }
        );
        g.io_mux_write(4 + 12, (1 << 12) | (3 << 10)).unwrap();
        assert_eq!(g.pin_out(3), PinOut::driven(false, 3));
        // UART0 TX (GPIO21) starts on function 0: U0TXD, idle high, whatever GPIO_ENABLE says.
        g.write(reg::ENABLE_W1TS, 1 << 21).unwrap();
        assert_eq!(g.pin_out(21).level, Some(true));
    }

    #[test]
    fn open_drain_releases_high_and_in_sel_is_stored() {
        let mut g = Gpio::new();
        g.write(reg::ENABLE_W1TS, 1 << 8).unwrap();
        g.write(reg::PIN + 4 * 8, pin_reg::PAD_DRIVER).unwrap();
        assert_eq!(g.read(reg::PIN + 4 * 8), Some(pin_reg::PAD_DRIVER));
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
    fn reset_pads_follow_the_datasheet() {
        let g = Gpio::new();
        for (p, &mux) in g.io_mux.iter().enumerate() {
            let ie = mux & io_mux::FUN_IE != 0;
            let wpu = mux & io_mux::FUN_WPU != 0;
            assert_eq!(ie, !matches!(p, 0 | 1 | 11 | 18 | 19 | 21), "IE on GPIO{p}");
            assert_eq!(wpu, matches!(p, 6 | 9 | 12..=17 | 20 | 21), "WPU on GPIO{p}");
            assert_eq!(mux & io_mux::FUN_WPD, 0);
        }
        assert_eq!(g.io_mux_read(4 + 4 * 9), Some(0x1b00), "GPIO9: GPIO, drive 2, IE, WPU");
        // Pins with input enable and pull-up read high; nothing else does.
        let high = (1 << 6) | (1 << 9) | (0x3f << 12) | (1 << 20);
        assert_eq!(g.read(reg::IN), Some(high));
        assert_eq!(g.read(reg::STATUS), Some(0));
    }

    #[test]
    fn input_follows_external_pads_gated_by_ie() {
        let mut g = Gpio::new();
        g.set_external(5, true);
        g.set_input(5, true);
        g.write(reg::IN, 0).unwrap();
        assert_eq!(g.read(reg::IN).unwrap() & (1 << 5), 1 << 5, "read-only");
        g.io_mux_write(4 + 4 * 5, 1 << 12).unwrap(); // FUN_IE off
        assert_eq!(g.read(reg::IN).unwrap() & (1 << 5), 0);
        assert_eq!(g.pad_levels() & (1 << 5), 1 << 5, "the pad is still high");
        // GPIO0 has no input enable after reset.
        g.set_external(0, true);
        g.set_input(0, true);
        assert_eq!(g.read(reg::IN).unwrap() & 1, 0);
    }

    #[test]
    fn becoming_external_raises_no_edge() {
        let mut g = Gpio::new();
        g.write(reg::PIN + 4 * 9, pin_cfg(3, 1)).unwrap(); // any edge
        assert_eq!(g.read(reg::IN).unwrap() >> 9 & 1, 1, "pulled up");
        g.set_external(9, true);
        assert_eq!(g.read(reg::STATUS), Some(0));
        g.set_input(9, false);
        assert_eq!(g.read(reg::STATUS), Some(1 << 9));
    }

    #[test]
    fn unlinked_pads_loop_back_outputs_and_pulls() {
        let mut g = Gpio::new();
        let bit = |g: &Gpio| g.read(reg::IN).unwrap() >> 4 & 1;
        assert_eq!(bit(&g), 0, "floating pad keeps its last level");
        g.io_mux_write(4 + 4 * 4, (1 << 12) | io_mux::FUN_IE | io_mux::FUN_WPU)
            .unwrap();
        assert_eq!(bit(&g), 1, "pulled up");
        g.write(reg::ENABLE_W1TS, 1 << 4).unwrap();
        assert_eq!(bit(&g), 0, "driving low wins over the pull");
        g.write(reg::OUT_W1TS, 1 << 4).unwrap();
        assert_eq!(bit(&g), 1);
        g.write(reg::ENABLE_W1TC, 1 << 4).unwrap();
        g.io_mux_write(4 + 4 * 4, (1 << 12) | io_mux::FUN_IE | io_mux::FUN_WPD)
            .unwrap();
        assert_eq!(bit(&g), 0, "pulled down");
        // An external pin ignores its own pulls: the circuit decides.
        g.set_external(4, true);
        g.set_input(4, true);
        assert_eq!(bit(&g), 1);
    }

    #[test]
    fn edge_interrupts_set_status_and_route_to_the_cpu() {
        let mut g = Gpio::new();
        g.set_external(7, true);
        g.write(reg::PIN + 4 * 7, pin_cfg(2, 1)).unwrap(); // falling, CPU
        g.set_input(7, true);
        assert_eq!(g.read(reg::STATUS), Some(0), "rising edge ignored");
        assert_eq!(g.irq(), (false, false));
        g.set_input(7, false);
        assert_eq!(g.read(reg::STATUS), Some(1 << 7));
        assert_eq!(g.read(reg::PCPU_INT), Some(1 << 7));
        assert_eq!(g.read(reg::PCPU_NMI_INT), Some(0));
        assert_eq!(g.irq(), (true, false));
        g.write(reg::STATUS_W1TC, 1 << 7).unwrap();
        assert_eq!(g.irq(), (false, false), "edges do not re-assert");
        // Any edge, routed to the NMI.
        g.write(reg::PIN + 4 * 7, pin_cfg(3, 2)).unwrap();
        g.set_input(7, true);
        assert_eq!(g.irq(), (false, true));
        assert_eq!(g.read(reg::PCPU_NMI_INT), Some(1 << 7));
        g.write(reg::STATUS, 0).unwrap();
        // Rising only.
        g.write(reg::PIN + 4 * 7, pin_cfg(1, 1)).unwrap();
        g.set_input(7, false);
        assert_eq!(g.read(reg::STATUS), Some(0));
        g.set_input(7, true);
        assert_eq!(g.read(reg::STATUS), Some(1 << 7));
        // Status bits can be set by software, and with no INT_ENA they are
        // pending but do not interrupt.
        g.write(reg::STATUS, 0).unwrap();
        g.write(reg::STATUS_W1TS, 1 << 3).unwrap();
        assert_eq!(g.read(reg::STATUS), Some(1 << 8 >> 5));
        assert_eq!(g.irq(), (false, false));
    }

    #[test]
    fn level_interrupts_re_assert_while_the_level_lasts() {
        let mut g = Gpio::new();
        g.set_external(10, true);
        g.set_input(10, true);
        g.write(reg::PIN + 4 * 10, pin_cfg(4, 1)).unwrap(); // low level
        assert_eq!(g.irq(), (false, false));
        g.set_input(10, false);
        assert!(g.irq().0);
        g.write(reg::STATUS_W1TC, 1 << 10).unwrap();
        assert!(g.irq().0, "still low");
        g.set_input(10, true);
        g.write(reg::STATUS_W1TC, 1 << 10).unwrap();
        assert!(!g.irq().0);
        g.write(reg::PIN + 4 * 10, pin_cfg(5, 1)).unwrap(); // high level
        assert!(g.irq().0, "already high");
    }

    #[test]
    fn peripheral_inputs_through_the_matrix() {
        let mut g = Gpio::new();
        g.set_external(20, true);
        g.set_input(20, false);
        assert_eq!(g.peripheral_input(6), None, "SEL = 0: IO MUX direct input");
        g.write(reg::FUNC_IN_SEL + 4 * 6, 20 | 1 << 6).unwrap();
        assert_eq!(g.peripheral_input(6), Some(false));
        g.set_input(20, true);
        assert_eq!(g.peripheral_input(6), Some(true));
        g.write(reg::FUNC_IN_SEL + 4 * 6, 20 | 1 << 5 | 1 << 6).unwrap();
        assert_eq!(g.peripheral_input(6), Some(false), "inverted");
        g.write(reg::FUNC_IN_SEL + 4 * 6, IN_CONST_ONE | 1 << 6).unwrap();
        assert_eq!(g.peripheral_input(6), Some(true));
        g.write(reg::FUNC_IN_SEL + 4 * 6, IN_CONST_ZERO | 1 << 6).unwrap();
        assert_eq!(g.peripheral_input(6), Some(false));
        assert_eq!(g.peripheral_input(128), None);
    }

    #[test]
    fn thevenin_per_strength_and_pulls() {
        assert_eq!(PinOut::driven(true, 2).thevenin(3.3), (3.3, 30.0));
        assert_eq!(PinOut::driven(false, 0).thevenin(3.3), (0.0, 120.0));
        assert_eq!(PinOut::HIZ.thevenin(3.3).1, 1e9);
        let up = PinOut {
            pull_up: true,
            ..PinOut::HIZ
        };
        assert_eq!(up.thevenin(3.3), (3.3, R_PULL));
        let down = PinOut {
            pull_down: true,
            ..PinOut::HIZ
        };
        assert_eq!(down.thevenin(3.3), (0.0, R_PULL));
        let (v, r) = PinOut { pull_down: true, ..up }.thevenin(3.3);
        assert!((v - 1.65).abs() < 1e-12 && (r - R_PULL / 2.0).abs() < 1e-9);
        // A driver low against the pull-up: a slight lift above 0 V.
        let (v, r) = PinOut {
            pull_up: true,
            ..PinOut::driven(false, 2)
        }
        .thevenin(3.3);
        assert!(v > 0.0 && v < 0.003 && r < 30.0, "{v} {r}");
    }
}
