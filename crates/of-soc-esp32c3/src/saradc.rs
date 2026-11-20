//! SAR ADC digital controller, one-time sampling (TRM chapter 34,
//! `APB_SARADC` at `0x6004_0000`).
//!
//! **Channels.** SAR ADC1 channels 0 to 4 are GPIO0 to GPIO4, SAR ADC2
//! channel 0 is GPIO5. Other channels (the internal calibration inputs)
//! read code 0.
//!
//! **One-time sampling.** Set `SARADC1_ONETIME_SAMPLE` (or `SARADC2_..`),
//! `ONETIME_CHANNEL` and `ONETIME_ATTEN`, then raise `ONETIME_START`. The
//! pad voltage is taken at that instant ([`AdcRequest`]: the chip engine
//! asks the analog engine for it, see [`crate::engine`]; without a link
//! the pad's static input from [`SarAdc::set_input`] is used, 0 V by
//! default). The conversion takes 25 SAR_CLK cycles (TRM 34.2.3.4); then
//! the code is in `SAR1DATA_STATUS` / `SAR2DATA_STATUS` and
//! `ADC1_DONE_INT_RAW` / `ADC2_DONE_INT_RAW` is set. SAR_CLK is
//! `APB_CLK` (80 MHz; `CLK_SEL` 1: PLL 240 MHz) divided by
//! `CLKM_DIV_NUM + 1 + CLKM_DIV_A / CLKM_DIV_B` (ESP-IDF's formula for the
//! digital controller clock) and by `SAR_CLK_DIV`: 4 MHz and 6.25 µs per
//! sample at reset values. The done interrupts drive source 43 (`APB_ADC`)
//! through `INT_ENA`.
//!
//! **Transfer.** Attenuation 0/1/2/3 (0, 2.5, 6 and 11 dB) gives the full
//! scales the datasheet's calibrated ranges come from. The TRM's nominal
//! `Vdata = Vref / k · data / 4095` (`Vref` 1100 mV by design, real chips
//! differ) is not used: Espressif's calibration (the eFuse calibration
//! code `cal_code` read for the voltage `cal_mv` = 400/550/750/1370 mV, then
//! a per-attenuation error polynomial, ESP-IDF
//! `curve_fitting_coefficients.c`) describes this chip better, so the code
//! for a voltage is the inverse of that calibration: the line voltage `x`
//! with `x - err(x) = Vin` (mV), then `round(x · cal_code / cal_mv)`,
//! clamped to 0..=4095. Reading it back with esp-hal's `AdcCalCurve` gives
//! the input to within a few millivolts ([`calibrated_mv`]); the
//! datasheet's calibrated error is ±10 mV (atten 0 to 2) and ±35 mV (atten
//! 3). The chip's calibration comes from the eFuse model
//! ([`crate::efuse::adc1_cal_code`]); ADC2 uses the same curve.
//!
//! Not modelled: noise, DNL and INL, the sampling capacitor's load on the
//! pad (the voltage is read, not loaded), the analog init code written over
//! the internal I2C bus (the factory one is assumed), the power-up
//! (`XPD_SAR`) and FSM wait times, timer-triggered pattern scanning with
//! DMA, filters, thresholds, the temperature sensor and the ADC2 errata
//! (ADC2 works here). Other registers read back what was written.

use of_core::SimTime;

use crate::efuse;

/// Register offsets from [`crate::map::APB_SARADC_BASE`].
pub mod reg {
    /// `APB_SARADC_CTRL` (`SAR_CLK_DIV` bits 14:7).
    pub const CTRL: u32 = 0x00;
    /// `APB_SARADC_ONETIME_SAMPLE`.
    pub const ONETIME_SAMPLE: u32 = 0x20;
    /// `APB_SARADC_SAR1DATA_STATUS` (ADC1 result, bits 16:0).
    pub const SAR1DATA_STATUS: u32 = 0x2c;
    /// `APB_SARADC_SAR2DATA_STATUS` (ADC2 result).
    pub const SAR2DATA_STATUS: u32 = 0x30;
    /// Interrupt enables.
    pub const INT_ENA: u32 = 0x40;
    /// Raw interrupt status.
    pub const INT_RAW: u32 = 0x44;
    /// Masked interrupt status.
    pub const INT_ST: u32 = 0x48;
    /// Write 1 to clear.
    pub const INT_CLR: u32 = 0x4c;
    /// `APB_SARADC_CLKM_CONF` (digital controller clock).
    pub const CLKM_CONF: u32 = 0x54;
    /// Last register of the block that is stored (`APB_SARADC_CALI`).
    pub const CALI: u32 = 0x60;
}

/// `APB_SARADC_ONETIME_SAMPLE` fields.
pub mod onetime {
    /// `SARADC1_ONETIME_SAMPLE`: sample with ADC1.
    pub const ADC1: u32 = 1 << 31;
    /// `SARADC2_ONETIME_SAMPLE`: sample with ADC2.
    pub const ADC2: u32 = 1 << 30;
    /// `ONETIME_START`.
    pub const START: u32 = 1 << 29;
    /// `ONETIME_CHANNEL` shift (4 bits).
    pub const CHANNEL_SHIFT: u32 = 25;
    /// `ONETIME_ATTEN` shift (2 bits).
    pub const ATTEN_SHIFT: u32 = 23;
}

/// Interrupt bits (`INT_RAW` and friends).
pub mod int {
    /// `ADC1_DONE`.
    pub const ADC1_DONE: u32 = 1 << 31;
    /// `ADC2_DONE`.
    pub const ADC2_DONE: u32 = 1 << 30;
}

/// SAR_CLK cycles per conversion (TRM 34.2.3.4).
pub const CYCLES_PER_SAMPLE: u64 = 25;
/// Pads with an ADC channel: GPIO0 to GPIO5.
pub const PADS: usize = 6;
/// Largest code.
pub const MAX_CODE: u16 = 4095;
/// Calibration voltage per attenuation (mV), as esp-hal and ESP-IDF use
/// with the eFuse calibration codes.
pub const CAL_MV: [f64; 4] = [400.0, 550.0, 750.0, 1370.0];

/// ESP-IDF's ESP32-C3 error-curve coefficients per attenuation (mV
/// polynomial in the line-calibrated voltage), as esp-hal 1.2.2 carries
/// them.
pub const CURVE: [&[f64]; 4] = [
    &[-0.225966470500043, -0.0007265418501948, 0.0000109410402681],
    &[0.4229623392600516, -0.0000731527490903, 0.0000088166562521],
    &[-1.017859239236435, -0.0097159265299153, 0.0000149794028038],
    &[
        -1.4912262772850453,
        -0.0228549975564099,
        0.0000356391935717,
        -0.0000000179964582,
        0.0000000000042046,
    ],
];

/// The pad of `unit` (0 = ADC1, 1 = ADC2) channel `channel`.
pub fn pad(unit: usize, channel: u32) -> Option<u8> {
    match (unit, channel) {
        (0, 0..=4) => Some(channel as u8),
        (1, 0) => Some(5),
        _ => None,
    }
}

fn err_mv(atten: usize, x: f64) -> f64 {
    CURVE[atten & 3].iter().rev().fold(0.0, |acc, c| acc * x + c)
}

/// The code the ADC gives for `volts` at attenuation `atten` (0..=3) on
/// a chip whose calibration code for [`CAL_MV`] is `cal_code`.
pub fn code_for(volts: f64, atten: usize, cal_code: u32) -> u16 {
    let atten = atten & 3;
    let target = volts * 1000.0;
    let per_code = CAL_MV[atten] / f64::from(cal_code.max(1));
    let top = f64::from(MAX_CODE) * per_code;
    let f = |x: f64| x - err_mv(atten, x);
    // NaN reads as 0 V.
    if target.is_nan() || target <= f(0.0) {
        return 0;
    }
    if target >= f(top) {
        return MAX_CODE;
    }
    // x - err(x) rises over the whole range (err' is below 0.2), so
    // bisection finds the line voltage.
    let (mut lo, mut hi) = (0.0, top);
    for _ in 0..60 {
        let mid = 0.5 * (lo + hi);
        if f(mid) < target {
            lo = mid;
        } else {
            hi = mid;
        }
    }
    (0.5 * (lo + hi) / per_code).round().clamp(0.0, f64::from(MAX_CODE)) as u16
}

/// What esp-hal 1.2.2's `AdcCalCurve` returns for `raw` (mV): the line
/// calibration `raw · gain >> 16` with `gain = cal_mv · 65536 / cal_code`,
/// minus the error polynomial in 2^52 fixed point, as it computes them.
pub fn calibrated_mv(raw: u16, atten: usize, cal_code: u32) -> u16 {
    let atten = atten & 3;
    let gain = CAL_MV[atten] as u32 * 65536 / cal_code.max(1);
    let line = (u32::from(raw) * gain / 65536) as u16;
    let err = if line == 0 {
        0
    } else {
        let mul = (1u64 << 52) as f64;
        let v = i64::from(line);
        let poly = CURVE[atten]
            .iter()
            .rev()
            .fold(0i64, |p, &c| p.wrapping_mul(v).wrapping_add((c * mul) as i64));
        (poly / (1i64 << 52)) as i32
    };
    (i32::from(line) - err) as u16
}

/// A conversion started: the pad's voltage is wanted at `at`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AdcRequest {
    /// When the sample is taken.
    pub at: SimTime,
    /// GPIO number of the pad.
    pub pad: u8,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct Conversion {
    unit: usize,
    pad: Option<u8>,
    atten: usize,
    done: SimTime,
    volts: Option<f64>,
}

const REGS: usize = (reg::CALI / 4 + 1) as usize;

/// The SAR ADC digital controller.
#[derive(Debug, Clone, PartialEq)]
pub struct SarAdc {
    regs: [u32; REGS],
    int_raw: u32,
    data: [u32; 2],
    pending: Vec<Conversion>,
    requests: Vec<AdcRequest>,
    inputs: [f64; PADS],
    cal_code: [u32; 4],
}

impl Default for SarAdc {
    fn default() -> Self {
        Self::new()
    }
}

/// Reset values (TRM / PAC) of the stored registers.
const RESETS: [(u32, u32); 13] = [
    (0x00, 0x4003_8240),
    (0x04, 0xa1fe),
    (0x0c, 0x00ff_0808),
    (0x20, 0x1a00_0000),
    (0x24, 0x0900),
    (0x28, 0x0374_0000),
    (0x34, 0x0003_ffed),
    (0x38, 0x0003_ffed),
    (0x50, 0xff),
    (0x54, 0x04),
    (0x58, 0x0001_8000),
    (0x5c, 0x4002),
    (0x60, 0x8000),
];

impl SarAdc {
    /// Reset state, with the eFuse model's ADC1 calibration and all pads
    /// at 0 V.
    pub fn new() -> Self {
        let mut regs = [0; REGS];
        for (off, v) in RESETS {
            regs[(off / 4) as usize] = v;
        }
        Self {
            regs,
            int_raw: 0,
            data: [0; 2],
            pending: Vec::new(),
            requests: Vec::new(),
            inputs: [0.0; PADS],
            cal_code: [0, 1, 2, 3].map(efuse::adc1_cal_code),
        }
    }

    /// Reset the registers; pad inputs belong to the circuit and stay.
    pub fn reset(&mut self) {
        let inputs = self.inputs;
        *self = Self::new();
        self.inputs = inputs;
    }

    /// Whether `off` is a register applied at the hart's time.
    pub fn is_reg(off: u32) -> bool {
        off <= reg::CALI
    }

    /// The calibration code of attenuation `atten` (see [`code_for`]).
    pub fn cal_code(&self, atten: usize) -> u32 {
        self.cal_code[atten & 3]
    }

    /// Set the voltage on GPIO `pad` (0..=5) used when no sample arrives
    /// for a conversion.
    pub fn set_input(&mut self, pad: u8, volts: f64) {
        if let Some(v) = self.inputs.get_mut(usize::from(pad)) {
            *v = volts;
        }
    }

    /// The voltage sampled for the conversion on `pad` that is waiting for
    /// one (it also becomes the pad's static input).
    pub fn set_sample(&mut self, pad: u8, volts: f64) {
        self.set_input(pad, volts);
        for c in self
            .pending
            .iter_mut()
            .filter(|c| c.pad == Some(pad) && c.volts.is_none())
        {
            c.volts = Some(volts);
        }
    }

    /// Conversions started since the last call.
    pub fn take_requests(&mut self) -> Vec<AdcRequest> {
        std::mem::take(&mut self.requests)
    }

    /// Interrupt output (source 43).
    pub fn irq(&self) -> bool {
        self.int_raw & self.regs[(reg::INT_ENA / 4) as usize] != 0
    }

    /// Duration of one conversion with the current clock settings.
    pub fn conversion_time(&self) -> SimTime {
        let ctrl = self.regs[(reg::CTRL / 4) as usize];
        let clkm = self.regs[(reg::CLKM_CONF / 4) as usize];
        let src = if (clkm >> 21) & 3 == 1 { 240e6 } else { 80e6 };
        let (num, b, a) = (clkm & 0xff, (clkm >> 8) & 0x3f, (clkm >> 14) & 0x3f);
        let frac = if b == 0 { 0.0 } else { f64::from(a) / f64::from(b) };
        let div = (f64::from(num) + 1.0 + frac) * f64::from(((ctrl >> 7) & 0xff).max(1));
        SimTime::from_secs_f64(CYCLES_PER_SAMPLE as f64 * div / src)
    }

    /// When the next conversion finishes.
    pub fn next_event(&self) -> Option<SimTime> {
        self.pending.iter().map(|c| c.done).min()
    }

    /// Finish the conversions due by `t`.
    pub fn advance(&mut self, t: SimTime) {
        let (due, rest): (Vec<_>, Vec<_>) = self.pending.iter().partition(|c| c.done <= t);
        self.pending = rest;
        for c in due {
            let volts = c.pad.map_or(0.0, |p| c.volts.unwrap_or(self.inputs[usize::from(p)]));
            let code = if c.pad.is_some() {
                code_for(volts, c.atten, self.cal_code[c.atten])
            } else {
                0
            };
            self.data[c.unit] = u32::from(code);
            self.int_raw |= if c.unit == 0 { int::ADC1_DONE } else { int::ADC2_DONE };
        }
    }

    /// Read a register.
    pub fn read(&self, off: u32) -> Option<u32> {
        match off {
            reg::SAR1DATA_STATUS => Some(self.data[0]),
            reg::SAR2DATA_STATUS => Some(self.data[1]),
            reg::INT_RAW => Some(self.int_raw),
            reg::INT_ST => Some(self.int_raw & self.regs[(reg::INT_ENA / 4) as usize]),
            reg::INT_CLR => Some(0),
            _ if Self::is_reg(off) && off.is_multiple_of(4) => Some(self.regs[(off / 4) as usize]),
            _ => None,
        }
    }

    /// Write a register at time `t` (call [`Self::advance`] to `t` first).
    pub fn write(&mut self, off: u32, v: u32, t: SimTime) -> Option<()> {
        if !Self::is_reg(off) || !off.is_multiple_of(4) {
            return None;
        }
        match off {
            reg::SAR1DATA_STATUS | reg::SAR2DATA_STATUS | reg::INT_RAW | reg::INT_ST => {}
            reg::INT_CLR => self.int_raw &= !v,
            reg::ONETIME_SAMPLE => {
                let old = self.regs[(off / 4) as usize];
                self.regs[(off / 4) as usize] = v;
                if v & onetime::START != 0 && old & onetime::START == 0 {
                    self.start(v, t);
                }
            }
            _ => self.regs[(off / 4) as usize] = v,
        }
        Some(())
    }

    fn start(&mut self, v: u32, t: SimTime) {
        let channel = (v >> onetime::CHANNEL_SHIFT) & 0xf;
        let atten = ((v >> onetime::ATTEN_SHIFT) & 3) as usize;
        let done = t.saturating_add(self.conversion_time());
        for (unit, bit) in [(0, onetime::ADC1), (1, onetime::ADC2)] {
            if v & bit == 0 {
                continue;
            }
            let pad = pad(unit, channel);
            self.pending.retain(|c| c.unit != unit);
            self.pending.push(Conversion {
                unit,
                pad,
                atten,
                done,
                volts: None,
            });
            if let Some(pad) = pad {
                self.requests.push(AdcRequest { at: t, pad });
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn calibrated_reading_recovers_the_input_within_a_few_millivolts() {
        let adc = SarAdc::new();
        // The datasheet's calibrated ranges.
        for (atten, top) in [(0, 750.0), (1, 1050.0), (2, 1300.0), (3, 2500.0)] {
            let cal = adc.cal_code(atten);
            let mut mv = 10.0;
            while mv <= top {
                let raw = code_for(mv / 1000.0, atten, cal);
                let back = f64::from(calibrated_mv(raw, atten, cal));
                assert!((back - mv).abs() <= 3.0, "atten {atten}: {mv} mV -> {raw} -> {back} mV");
                mv += 7.3;
            }
        }
    }

    #[test]
    fn codes_saturate_and_grow_with_the_input() {
        let cal = efuse::adc1_cal_code(3);
        assert_eq!(code_for(-0.1, 3, cal), 0);
        assert_eq!(code_for(0.0, 3, cal), 0);
        assert_eq!(code_for(3.3, 3, cal), MAX_CODE);
        let mut last = 0;
        for i in 0..=330 {
            let c = code_for(f64::from(i) * 0.01, 3, cal);
            assert!(c >= last);
            last = c;
        }
        // Full scale near 2.8 V at 11 dB on this chip.
        let full = (0..3300).find(|&mv| code_for(f64::from(mv) / 1000.0, 3, cal) == MAX_CODE);
        assert!(full.is_some_and(|mv| (2_750..2_950).contains(&mv)), "{full:?}");
        // A line voltage of 1370 mV reads the calibration code.
        let x = 1370.0;
        assert_eq!(code_for((x - err_mv(3, x)) / 1000.0, 3, cal), cal as u16);
    }

    #[test]
    fn one_time_sample_with_timing_interrupt_and_request() {
        let mut adc = SarAdc::new();
        adc.set_input(2, 1.0);
        let t = SimTime::from_us(10);
        assert_eq!(adc.conversion_time(), SimTime::from_ns(6_250), "reset clocks: 4 MHz");
        let cfg = onetime::ADC1 | (2 << onetime::CHANNEL_SHIFT) | (3 << onetime::ATTEN_SHIFT);
        adc.write(reg::ONETIME_SAMPLE, cfg, t).unwrap();
        assert!(adc.take_requests().is_empty());
        adc.write(reg::ONETIME_SAMPLE, cfg | onetime::START, t).unwrap();
        assert_eq!(adc.take_requests(), vec![AdcRequest { at: t, pad: 2 }]);
        let done = t.saturating_add(SimTime::from_ns(6_250));
        assert_eq!(adc.next_event(), Some(done));
        adc.set_sample(2, 1.5);
        adc.advance(SimTime(done.0 - 1));
        assert_eq!(adc.read(reg::INT_RAW), Some(0));
        adc.advance(done);
        assert_eq!(adc.read(reg::INT_RAW), Some(int::ADC1_DONE));
        let raw = adc.read(reg::SAR1DATA_STATUS).unwrap() as u16;
        assert_eq!(
            raw,
            code_for(1.5, 3, adc.cal_code(3)),
            "the sample, not the static input"
        );
        assert!(!adc.irq());
        adc.write(reg::INT_ENA, int::ADC1_DONE, done).unwrap();
        assert!(adc.irq());
        adc.write(reg::INT_CLR, int::ADC1_DONE, done).unwrap();
        assert!(!adc.irq());
        // START held high starts nothing; clear and set again.
        adc.write(reg::ONETIME_SAMPLE, cfg | onetime::START, done).unwrap();
        assert!(adc.take_requests().is_empty());
        adc.write(reg::ONETIME_SAMPLE, cfg, done).unwrap();
        adc.write(reg::ONETIME_SAMPLE, cfg | onetime::START, done).unwrap();
        adc.advance(SimTime::from_us(100));
        assert_eq!(
            adc.read(reg::SAR1DATA_STATUS),
            Some(u32::from(code_for(1.5, 3, adc.cal_code(3))))
        );
        // A faster clock: SAR_CLK_DIV 2 and CLKM_DIV_NUM 1 give 20 MHz.
        adc.write(reg::CTRL, (0x4003_8240 & !(0xff << 7)) | (2 << 7), done)
            .unwrap();
        adc.write(reg::CLKM_CONF, 1, done).unwrap();
        assert_eq!(adc.conversion_time(), SimTime::from_ns(1_250));
    }

    #[test]
    fn adc2_and_internal_channels() {
        let mut adc = SarAdc::new();
        adc.set_input(5, 0.5);
        let t = SimTime::ZERO;
        adc.write(reg::ONETIME_SAMPLE, onetime::ADC2 | onetime::START, t)
            .unwrap();
        adc.advance(SimTime::from_us(10));
        assert_eq!(
            adc.read(reg::SAR2DATA_STATUS),
            Some(u32::from(code_for(0.5, 0, adc.cal_code(0))))
        );
        assert_eq!(adc.read(reg::INT_RAW), Some(int::ADC2_DONE));
        let ch15 = onetime::ADC1 | (15 << onetime::CHANNEL_SHIFT);
        adc.write(reg::ONETIME_SAMPLE, ch15, t).unwrap();
        adc.write(reg::ONETIME_SAMPLE, ch15 | onetime::START, t).unwrap();
        assert!(adc.take_requests().iter().all(|r| r.pad == 5), "channel 15 has no pad");
        adc.advance(SimTime::from_us(20));
        assert_eq!(adc.read(reg::SAR1DATA_STATUS), Some(0));
        assert_eq!(adc.read(0x3fc), None, "DATE is not modelled");
    }
}
