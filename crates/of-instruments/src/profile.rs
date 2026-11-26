//! Real-world instrument profiles (engine E5f, RFC 0004 "Real-world profiles").
//!
//! A profile models only deterministic effects: the display (ranges,
//! resolution, autorange and `OL`), the ± accuracy band shown next to a
//! reading (never applied as a random error, decision D2), the scope's
//! vertical quantisation and the elements an instrument adds to the circuit
//! ([`LoadElement`]). The figures live in the parts library as
//! `parts/instruments/<vendor>.<model>/instrument.toml` (CC-BY-SA-4.0, every
//! table cites its source) and are embedded at build time, so the web build
//! has them too; this module holds only behaviour and the loader ([`parse`]).

use std::fmt;
use std::sync::OnceLock;

use serde::Deserialize;

/// The built-in profile files, embedded from `parts/instruments/`.
pub const BUILTIN_FILES: &[(&str, &str)] = &[
    (
        "fluke.87v",
        include_str!("../../../parts/instruments/fluke.87v/instrument.toml"),
    ),
    (
        "rigol.ds1054z",
        include_str!("../../../parts/instruments/rigol.ds1054z/instrument.toml"),
    ),
    (
        "rigol.dp832",
        include_str!("../../../parts/instruments/rigol.dp832/instrument.toml"),
    ),
    (
        "saleae.logic8",
        include_str!("../../../parts/instruments/saleae.logic8/instrument.toml"),
    ),
];

/// What a real-world instrument adds to the circuit.
#[derive(Debug, Clone, PartialEq)]
pub enum LoadElement {
    /// Shunt from `net` to `reference` (meter or probe input): `ohms` in
    /// parallel with `farads` (0 for none).
    Shunt {
        /// Measured net.
        net: String,
        /// Reference net (ground or the second voltmeter lead).
        reference: String,
        /// Input resistance.
        ohms: f64,
        /// Input capacitance (0 when the source gives no nominal value).
        farads: f64,
    },
    /// Series element at a component pin (ammeter burden): the pin moves to
    /// a new net and `ohms` joins it to the pin's original net.
    Series {
        /// Component reference.
        refdes: String,
        /// Pin name.
        pin: String,
        /// Burden resistance.
        ohms: f64,
    },
}

/// One display range of a multimeter function.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DmmRange {
    /// Range label as printed, for example `"600.0 mV"`.
    pub label: String,
    /// Last-digit resolution in SI units.
    pub resolution: f64,
    /// Accuracy, % of reading.
    pub pct: f64,
    /// Accuracy, counts of the last digit.
    pub counts: u32,
    /// Largest magnitude shown before the next range (or `OL`), the
    /// range's printed value.
    pub full_scale: f64,
    /// Where the figures come from.
    pub source: String,
}

/// A multimeter current input (jack and rotary position).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AmpInput {
    /// µA position (600.0 µA, 6000 µA).
    Micro,
    /// mA position on the mA/µA jack (60.00 mA, 400.0 mA).
    Milli,
    /// A jack (6.000 A, 10.00 A).
    Amps,
}

impl AmpInput {
    /// Parse `"uA"`, `"mA"` or `"A"`.
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "uA" | "\u{b5}A" => Some(Self::Micro),
            "mA" => Some(Self::Milli),
            "A" => Some(Self::Amps),
            _ => None,
        }
    }
}

/// One current function: its ranges and typical burden.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AmpFunction {
    /// Typical burden voltage per ampere (V/A), i.e. the series resistance.
    pub burden_ohms: f64,
    /// Where the burden comes from.
    pub source: String,
    /// Ranges, smallest first.
    pub ranges: Vec<DmmRange>,
}

/// The volts input of a multimeter.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DmmInput {
    /// Input resistance on the volts functions.
    pub ohms: f64,
    /// Where it comes from.
    pub source: String,
}

/// Extra % of reading above a resistance (the 87V adds 0.5 % above 30 MΩ).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OhmsAdder {
    /// Threshold (Ω).
    pub above: f64,
    /// Extra % of reading.
    pub pct: f64,
    /// Where it comes from.
    pub source: String,
}

/// A handheld multimeter (DC volts, DC amps and resistance display).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DmmProfile {
    /// Profile id, `<vendor>.<model>` (from `[instrument] id`).
    #[serde(skip)]
    pub id: String,
    /// Display name (from `[instrument] name`).
    #[serde(skip)]
    pub name: String,
    /// Display counts (informational; the ranges carry their own full scale).
    pub max_counts: u32,
    /// Where the count comes from.
    pub counts_source: String,
    /// Volts input.
    pub input: DmmInput,
    /// DC volts ranges, smallest first.
    pub dc_v: Vec<DmmRange>,
    /// Resistance ranges, smallest first (display and band only; the
    /// meter's test current is not simulated yet).
    pub ohms: Vec<DmmRange>,
    /// Extra resistance band above a threshold, if any.
    pub ohms_adder: Option<OhmsAdder>,
    /// µA function.
    pub micro: AmpFunction,
    /// mA function.
    pub milli: AmpFunction,
    /// A function.
    pub amps: AmpFunction,
}

/// What a meter shows.
#[derive(Debug, Clone, PartialEq)]
pub struct Display {
    /// The value rounded to the range's last digit (`None` on overload).
    pub value: Option<f64>,
    /// ± accuracy band in SI units (`None` on overload).
    pub band: Option<f64>,
    /// The range used.
    pub range: DmmRange,
}

impl Display {
    /// True when the input is beyond the highest range (`OL`).
    pub fn overload(&self) -> bool {
        self.value.is_none()
    }
}

impl fmt::Display for Display {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (self.value, self.band) {
            (Some(v), Some(b)) => {
                // In the range's unit: "22.98 ± 0.09 mA".
                let unit = self.range.label.rsplit(' ').next().unwrap_or("");
                let scale = match unit.chars().next() {
                    Some('m') => 1e-3,
                    Some('k') => 1e3,
                    Some('M') => 1e6,
                    Some('\u{b5}') => 1e-6,
                    _ => 1.0,
                };
                let digits = digits(self.range.resolution / scale);
                write!(f, "{:.digits$} ± {:.digits$} {unit}", v / scale, b / scale)
            }
            _ => write!(f, "OL ({})", self.range.label),
        }
    }
}

/// Decimal places of a resolution such as 1e-3 (3); 0 for 1 or more.
fn digits(resolution: f64) -> usize {
    let d = -resolution.log10().round();
    if d > 0.0 { d as usize } else { 0 }
}

/// Autorange over `ranges`: the smallest range whose full scale holds
/// `value` once rounded to its last digit; above the last range the display
/// is `OL` on it. (No hysteresis: the real meter's range-change points are
/// not in the manual.)
pub fn autorange(ranges: &[DmmRange], value: f64) -> Display {
    let last = ranges.last().expect("a function has at least one range").clone();
    for r in ranges {
        let shown = (value / r.resolution).round() * r.resolution;
        if shown.abs() <= r.full_scale + r.resolution * 1e-9 {
            let band = r.pct / 100.0 * shown.abs() + f64::from(r.counts) * r.resolution;
            return Display {
                value: Some(shown),
                band: Some(band),
                range: r.clone(),
            };
        }
    }
    Display {
        value: None,
        band: None,
        range: last,
    }
}

impl DmmProfile {
    /// The current function behind `input`.
    pub fn amp_function(&self, input: AmpInput) -> &AmpFunction {
        match input {
            AmpInput::Micro => &self.micro,
            AmpInput::Milli => &self.milli,
            AmpInput::Amps => &self.amps,
        }
    }

    /// DC volts display of `volts`.
    pub fn dc_volts(&self, volts: f64) -> Display {
        autorange(&self.dc_v, volts)
    }

    /// DC amps display of `amps` on `input`.
    pub fn dc_amps(&self, input: AmpInput, amps: f64) -> Display {
        autorange(&self.amp_function(input).ranges, amps)
    }

    /// Resistance display of `ohms` (autoranged, `OL` above the top range).
    pub fn resistance(&self, ohms: f64) -> Display {
        let mut d = autorange(&self.ohms, ohms);
        if let Some(adder) = &self.ohms_adder
            && let (Some(v), Some(b)) = (d.value, d.band.as_mut())
            && v.abs() > adder.above
        {
            *b += adder.pct / 100.0 * v.abs();
        }
        d
    }

    /// The voltmeter's input between `net` and `reference`.
    pub fn volts_load(&self, net: &str, reference: &str) -> LoadElement {
        LoadElement::Shunt {
            net: net.into(),
            reference: reference.into(),
            ohms: self.input.ohms,
            farads: 0.0,
        }
    }

    /// The ammeter's burden in series with `refdes.pin`.
    pub fn amps_load(&self, input: AmpInput, refdes: &str, pin: &str) -> LoadElement {
        LoadElement::Series {
            refdes: refdes.into(),
            pin: pin.into(),
            ohms: self.amp_function(input).burden_ohms,
        }
    }
}

/// A scope's ADC.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeAdc {
    /// ADC codes per vertical division (DS1054Z: `YINCrement = VerticalScale/25`).
    pub codes_per_div: f64,
    /// Code of the reference (0 V at zero offset).
    pub reference_code: i32,
    /// Highest ADC code (8 bits: 255).
    pub max_code: i32,
    /// Where it comes from.
    pub source: String,
}

/// A scope's input (1X probe).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeInput {
    /// Input resistance.
    pub ohms: f64,
    /// Input capacitance (nominal).
    pub farads: f64,
    /// Where it comes from.
    pub source: String,
}

/// A scope's DC accuracy.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeAccuracy {
    /// Divisions taken as "full scale" for the gain accuracy.
    pub screen_divs: f64,
    /// Gain accuracy below [`Self::gain_low_below_volts_div`] (% of full scale).
    pub gain_pct_low: f64,
    /// Gain accuracy otherwise (% of full scale).
    pub gain_pct: f64,
    /// V/div below which [`Self::gain_pct_low`] applies.
    pub gain_low_below_volts_div: f64,
    /// Offset accuracy, divisions.
    pub offset_divs: f64,
    /// Offset accuracy, volts.
    pub offset_volts: f64,
    /// Offset accuracy, % of the offset.
    pub offset_pct: f64,
    /// Where it comes from.
    pub source: String,
}

/// An oscilloscope's analog front end.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ScopeProfile {
    /// Profile id.
    #[serde(skip)]
    pub id: String,
    /// Display name.
    #[serde(skip)]
    pub name: String,
    /// ADC.
    pub adc: ScopeAdc,
    /// Input.
    pub input: ScopeInput,
    /// DC accuracy.
    pub accuracy: ScopeAccuracy,
}

impl ScopeProfile {
    /// Volts per code at `volts_div`.
    pub fn step(&self, volts_div: f64) -> f64 {
        volts_div / self.adc.codes_per_div
    }

    /// `value` as the ADC records it at `volts_div` and vertical `offset`
    /// (the trace shows `value + offset`): rounded to a code and clipped to
    /// the code span, then converted back (`(code - YORigin - YREFerence) × YINCrement`).
    pub fn quantise(&self, value: f64, volts_div: f64, offset: f64) -> f64 {
        let step = self.step(volts_div);
        let reference = f64::from(self.adc.reference_code);
        let code = ((value + offset) / step).round() + reference;
        let code = code.clamp(0.0, f64::from(self.adc.max_code));
        (code - reference) * step - offset
    }

    /// ± band of a DC reading: gain accuracy of full scale (`screen_divs` ×
    /// V/div) plus the offset accuracy (divisions, volts and % of offset).
    pub fn band(&self, volts_div: f64, offset: f64) -> f64 {
        let a = &self.accuracy;
        let gain = if volts_div < a.gain_low_below_volts_div {
            a.gain_pct_low
        } else {
            a.gain_pct
        };
        gain / 100.0 * a.screen_divs * volts_div
            + a.offset_divs * volts_div
            + a.offset_volts
            + a.offset_pct / 100.0 * offset.abs()
    }

    /// The band without its absolute-volt terms (offset volts, % of offset):
    /// for a trace whose probe ratio is not modelled, such as the desktop's
    /// LED current, in the trace's own unit.
    pub fn relative_band(&self, per_div: f64) -> f64 {
        let a = &self.accuracy;
        let gain = if per_div < a.gain_low_below_volts_div {
            a.gain_pct_low
        } else {
            a.gain_pct
        };
        gain / 100.0 * a.screen_divs * per_div + a.offset_divs * per_div
    }

    /// The probe's input on `net` to ground.
    pub fn load(&self, net: &str, ground: &str) -> LoadElement {
        LoadElement::Shunt {
            net: net.into(),
            reference: ground.into(),
            ohms: self.input.ohms,
            farads: self.input.farads,
        }
    }
}

/// One output channel of a bench supply's readback.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PsuChannel {
    /// Channel name (`CH1`).
    pub name: String,
    /// Rated volts.
    pub max_volts: f64,
    /// Rated amps.
    pub max_amps: f64,
    /// Readback resolution (V).
    pub volts_resolution: f64,
    /// Readback resolution (A).
    pub amps_resolution: f64,
    /// Readback accuracy, % of output (V).
    pub volts_pct: f64,
    /// Readback accuracy, offset (V).
    pub volts_offset: f64,
    /// Readback accuracy, % of output (A).
    pub amps_pct: f64,
    /// Readback accuracy, offset (A).
    pub amps_offset: f64,
    /// Where the figures come from.
    pub source: String,
}

/// A bench supply's readback.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PsuProfile {
    /// Profile id.
    #[serde(skip)]
    pub id: String,
    /// Display name.
    #[serde(skip)]
    pub name: String,
    /// Channels CH1, CH2, ...
    pub channels: Vec<PsuChannel>,
}

/// A supply readback: rounded values and their ± bands.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Readback {
    /// Volts as read back.
    pub volts: f64,
    /// ± band (V).
    pub volts_band: f64,
    /// Amps as read back.
    pub amps: f64,
    /// ± band (A).
    pub amps_band: f64,
}

impl PsuChannel {
    /// What the supply reports for `volts` and `amps` at its terminals.
    pub fn readback(&self, volts: f64, amps: f64) -> Readback {
        let v = (volts / self.volts_resolution).round() * self.volts_resolution;
        let a = (amps / self.amps_resolution).round() * self.amps_resolution;
        Readback {
            volts: v,
            volts_band: self.volts_pct / 100.0 * v.abs() + self.volts_offset,
            amps: a,
            amps_band: self.amps_pct / 100.0 * a.abs() + self.amps_offset,
        }
    }
}

/// A logic analyser's input thresholds.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicThresholds {
    /// Highest voltage read as low.
    pub vil: f64,
    /// Lowest voltage read as high.
    pub vih: f64,
    /// Where they come from.
    pub source: String,
}

/// The highest sample rate for up to `channels` active channels.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicRateLimit {
    /// Active channels.
    pub channels: usize,
    /// Highest rate (Hz).
    pub max_rate: f64,
    /// Where it comes from.
    pub source: String,
}

/// A logic analyser's digital inputs.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogicProfile {
    /// Profile id.
    #[serde(skip)]
    pub id: String,
    /// Display name.
    #[serde(skip)]
    pub name: String,
    /// Digital channels.
    pub channels: usize,
    /// Where the channel count comes from.
    pub channels_source: String,
    /// Selectable sample rates (Hz).
    pub rates: Vec<f64>,
    /// Where the rates come from.
    pub rates_source: String,
    /// Input (resistance and capacitance to ground).
    pub input: ScopeInput,
    /// Thresholds.
    pub thresholds: LogicThresholds,
    /// Rate limits by active channel count.
    pub rate_limits: Vec<LogicRateLimit>,
}

impl LogicProfile {
    /// The highest listed rate at or below `requested` that `channels`
    /// active channels allow.
    pub fn rate(&self, requested: f64, channels: usize) -> Option<f64> {
        let cap = self
            .rate_limits
            .iter()
            .filter(|l| channels <= l.channels)
            .map(|l| l.max_rate)
            .fold(None, |m: Option<f64>, r| Some(m.map_or(r, |m| m.max(r))))?;
        self.rates
            .iter()
            .copied()
            .filter(|&r| r <= requested && r <= cap)
            .fold(None, |m: Option<f64>, r| Some(m.map_or(r, |m| m.max(r))))
    }

    /// A fresh input with these thresholds.
    pub fn input(&self) -> crate::logic::LogicInput {
        crate::logic::LogicInput::new(self.thresholds.vil, self.thresholds.vih)
    }

    /// The probe's input on `net` to ground.
    pub fn load(&self, net: &str, ground: &str) -> LoadElement {
        LoadElement::Shunt {
            net: net.into(),
            reference: ground.into(),
            ohms: self.input.ohms,
            farads: self.input.farads,
        }
    }
}

/// A profile by id.
#[derive(Debug, Clone, PartialEq)]
pub enum Profile {
    /// A multimeter.
    Dmm(DmmProfile),
    /// A scope.
    Scope(ScopeProfile),
    /// A bench-supply readback.
    Psu(PsuProfile),
    /// A logic analyser.
    Logic(LogicProfile),
}

impl Profile {
    /// The profile id.
    pub fn id(&self) -> &str {
        match self {
            Profile::Dmm(p) => &p.id,
            Profile::Scope(p) => &p.id,
            Profile::Psu(p) => &p.id,
            Profile::Logic(p) => &p.id,
        }
    }

    /// The display name.
    pub fn name(&self) -> &str {
        match self {
            Profile::Dmm(p) => &p.name,
            Profile::Scope(p) => &p.name,
            Profile::Psu(p) => &p.name,
            Profile::Logic(p) => &p.name,
        }
    }

    /// The instrument kind (`[[instrument]] kind`) it fits, if any.
    pub fn kind(&self) -> Option<&'static str> {
        match self {
            Profile::Dmm(_) => Some("multimeter"),
            Profile::Scope(_) => Some("scope"),
            Profile::Psu(_) => None,
            Profile::Logic(_) => Some("logic"),
        }
    }
}

/// An `instrument.toml` that does not parse.
#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    /// TOML syntax or a missing or unknown field (every table needs its `source`).
    #[error("invalid instrument.toml: {0}")]
    Toml(String),
    /// `[instrument] kind` without its matching table.
    #[error("instrument.toml: kind '{0}' needs a [{0}] table")]
    Kind(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Header {
    id: String,
    kind: String,
    name: String,
    #[allow(dead_code)]
    verified: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct File {
    instrument: Header,
    multimeter: Option<DmmProfile>,
    scope: Option<ScopeProfile>,
    psu: Option<PsuProfile>,
    logic: Option<LogicProfile>,
}

/// Parse an `instrument.toml` document.
pub fn parse(text: &str) -> Result<Profile, ProfileError> {
    let f: File = toml::from_str(text).map_err(|e| ProfileError::Toml(e.to_string()))?;
    let Header { id, kind, name, .. } = f.instrument;
    let tables = usize::from(f.multimeter.is_some())
        + usize::from(f.scope.is_some())
        + usize::from(f.psu.is_some())
        + usize::from(f.logic.is_some());
    if tables != 1 {
        return Err(ProfileError::Kind(kind));
    }
    match (kind.as_str(), f.multimeter, f.scope, f.psu, f.logic) {
        ("multimeter", Some(mut p), ..) => {
            (p.id, p.name) = (id, name);
            Ok(Profile::Dmm(p))
        }
        ("scope", _, Some(mut p), ..) => {
            (p.id, p.name) = (id, name);
            Ok(Profile::Scope(p))
        }
        ("psu", _, _, Some(mut p), _) => {
            (p.id, p.name) = (id, name);
            Ok(Profile::Psu(p))
        }
        ("logic", _, _, _, Some(mut p)) => {
            (p.id, p.name) = (id, name);
            Ok(Profile::Logic(p))
        }
        _ => Err(ProfileError::Kind(kind)),
    }
}

/// Every built-in profile, parsed once from [`BUILTIN_FILES`].
pub fn builtins() -> &'static [Profile] {
    static ALL: OnceLock<Vec<Profile>> = OnceLock::new();
    ALL.get_or_init(|| {
        BUILTIN_FILES
            .iter()
            .map(|(id, text)| {
                let p = parse(text).unwrap_or_else(|e| panic!("built-in profile {id}: {e}"));
                assert_eq!(p.id(), *id, "built-in profile id");
                p
            })
            .collect()
    })
}

/// The built-in profile `id` (`fluke.87v`, `rigol.ds1054z`, `rigol.dp832`).
pub fn lookup(id: &str) -> Option<&'static Profile> {
    builtins().iter().find(|p| p.id() == id)
}

/// The built-in multimeter profiles.
pub fn multimeters() -> impl Iterator<Item = &'static DmmProfile> {
    builtins().iter().filter_map(|p| match p {
        Profile::Dmm(d) => Some(d),
        _ => None,
    })
}

/// The built-in scope profiles.
pub fn scopes() -> impl Iterator<Item = &'static ScopeProfile> {
    builtins().iter().filter_map(|p| match p {
        Profile::Scope(s) => Some(s),
        _ => None,
    })
}

/// The built-in logic-analyser profiles.
pub fn logics() -> impl Iterator<Item = &'static LogicProfile> {
    builtins().iter().filter_map(|p| match p {
        Profile::Logic(l) => Some(l),
        _ => None,
    })
}

/// The built-in supply-readback profiles.
pub fn supplies() -> impl Iterator<Item = &'static PsuProfile> {
    builtins().iter().filter_map(|p| match p {
        Profile::Psu(s) => Some(s),
        _ => None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fluke() -> &'static DmmProfile {
        multimeters().find(|p| p.id == "fluke.87v").unwrap()
    }

    fn scope() -> &'static ScopeProfile {
        scopes().find(|p| p.id == "rigol.ds1054z").unwrap()
    }

    fn psu() -> &'static PsuProfile {
        supplies().find(|p| p.id == "rigol.dp832").unwrap()
    }

    fn close(a: f64, b: f64) -> bool {
        (a - b).abs() <= 1e-12_f64.max(1e-9 * b.abs())
    }

    #[test]
    fn fluke_volts_autorange_round_and_band() {
        // 1.6492 V on the 6.000 V range: 1.649 V ± (0.05 % × 1.649 + 1 × 1 mV).
        let d = fluke().dc_volts(1.649_2);
        assert_eq!(d.range.label, "6.000 V");
        assert!(close(d.value.unwrap(), 1.649), "{d}");
        assert!(close(d.band.unwrap(), 0.0005 * 1.649 + 0.001), "{d}");
        assert_eq!(d.to_string(), "1.649 ± 0.002 V");
        // 0.25 V stays on 600.0 mV: 250.0 mV ± (0.1 % + 1 count).
        let d = fluke().dc_volts(0.25);
        assert_eq!(d.range.label, "600.0 mV");
        assert!(close(d.band.unwrap(), 0.001 * 0.25 + 1e-4));
        // 600.04 mV rounds to 600.0 mV (6000 counts) and still fits.
        assert_eq!(fluke().dc_volts(0.600_04).range.label, "600.0 mV");
        assert_eq!(fluke().dc_volts(0.600_06).range.label, "6.000 V");
        // Negative readings use the magnitude.
        assert!(close(fluke().dc_volts(-3.3).value.unwrap(), -3.3));
        // Above 1000 V: OL.
        let ol = fluke().dc_volts(1500.0);
        assert!(ol.overload());
        assert_eq!(ol.to_string(), "OL (1000 V)");
    }

    #[test]
    fn fluke_amps_use_the_chosen_input() {
        // 22.98 mA on mA: 60.00 mA range, ±(0.2 % + 4 counts of 10 µA).
        let d = fluke().dc_amps(AmpInput::Milli, 22.98e-3);
        assert_eq!(d.range.label, "60.00 mA");
        assert!(close(d.value.unwrap(), 22.98e-3));
        assert!(close(d.band.unwrap(), 0.002 * 22.98e-3 + 4e-5));
        assert_eq!(d.to_string(), "22.98 ± 0.09 mA");
        assert_eq!(
            fluke().dc_amps(AmpInput::Micro, 151.2e-6).to_string(),
            "151.2 ± 0.7 \u{b5}A"
        );
        assert_eq!(fluke().dc_amps(AmpInput::Milli, 0.1).range.label, "400.0 mA");
        // 400.0 mA is the top of the mA input (Table 13 note 5): 0.45 A is OL.
        assert!(fluke().dc_amps(AmpInput::Milli, 0.45).overload());
        assert!(fluke().dc_amps(AmpInput::Micro, 0.01).overload());
        assert_eq!(fluke().dc_amps(AmpInput::Amps, 2.0).range.label, "6.000 A");
        // Burden: 1.8 mV/mA -> 5 mA drops 9 mV.
        let LoadElement::Series { ohms, .. } = fluke().amps_load(AmpInput::Milli, "R1", "1") else {
            panic!()
        };
        assert!(close(ohms * 5e-3, 9e-3));
        assert_eq!(AmpInput::parse("mA"), Some(AmpInput::Milli));
        assert_eq!(AmpInput::parse("ma"), None);
    }

    #[test]
    fn fluke_resistance_ranges_and_the_30_megohm_adder() {
        let d = fluke().resistance(330.0);
        assert_eq!(d.to_string(), "330.0 \u{b1} 0.9 \u{3a9}");
        assert!(close(d.band.unwrap(), 0.002 * 330.0 + 0.2));
        assert_eq!(fluke().resistance(10e3).to_string(), "10.00 \u{b1} 0.03 k\u{3a9}");
        let hi = fluke().resistance(40e6);
        assert!(close(hi.band.unwrap(), 0.015 * 40e6 + 30e3), "{hi}");
        assert!(fluke().resistance(60e6).overload());
    }

    #[test]
    fn ds1054z_step_is_a_25th_of_a_division_and_clips_at_8_bits() {
        assert!(close(scope().step(1.0), 0.04));
        // 1 V/div: 0.05 V rounds to 1 code = 40 mV, 0.07 V to 2 codes = 80 mV.
        assert!(close(scope().quantise(0.05, 1.0, 0.0), 0.04));
        assert!(close(scope().quantise(0.07, 1.0, 0.0), 0.08));
        // Codes 0..=255 around 127: +128 codes (5.12 div) up, -127 (5.08 div) down.
        assert!(close(scope().quantise(100.0, 1.0, 0.0), 128.0 * 0.04));
        assert!(close(scope().quantise(-100.0, 1.0, 0.0), -127.0 * 0.04));
        // A -3 V offset moves the window: 3.3 V at 0.5 V/div reads 3.3 V.
        assert!(close(scope().quantise(3.3, 0.5, -3.0), 3.3));
        // Band at 1 V/div, no offset: 3 % of 8 V + 0.1 div + 2 mV = 0.342 V.
        assert!(close(scope().band(1.0, 0.0), 0.342));
        // Below 10 mV/div the gain term is 4 %.
        assert!(close(scope().band(5e-3, 0.0), 0.04 * 0.04 + 5e-4 + 2e-3));
    }

    #[test]
    fn dp832_readback_rounds_and_bands_per_channel() {
        let ch1 = psu().channels[0].clone();
        let r = ch1.readback(3.297_7, 0.022_98);
        assert!(close(r.volts, 3.30) && close(r.amps, 0.023), "{r:?}");
        assert!(close(r.volts_band, 0.0005 * 3.30 + 0.010));
        assert!(close(r.amps_band, 0.0015 * 0.023 + 0.005));
        let ch3 = psu().channels[2].clone();
        assert_eq!((ch3.max_volts, ch3.max_amps), (5.0, 3.0));
        assert!(close(ch3.readback(3.3, 0.0).volts_band, 0.001 * 3.3 + 0.005));
    }

    #[test]
    fn profiles_are_found_by_id_and_kind() {
        assert_eq!(lookup("fluke.87v").and_then(|p| p.kind()), Some("multimeter"));
        assert_eq!(lookup("rigol.ds1054z").and_then(|p| p.kind()), Some("scope"));
        assert_eq!(lookup("rigol.dp832").map(|p| p.kind()), Some(None));
        assert_eq!(lookup("saleae.logic8").and_then(|p| p.kind()), Some("logic"));
        assert!(lookup("acme.x1").is_none());
        assert_eq!(lookup("fluke.87v").unwrap().name(), "Fluke 87V");
    }

    #[test]
    fn every_table_needs_its_source() {
        let fluke = BUILTIN_FILES[0].1;
        let missing = fluke.replacen("source = \"[FL] Table 17, p. 47\"\n", "", 1);
        assert_ne!(missing, fluke);
        assert!(matches!(parse(&missing), Err(ProfileError::Toml(_))));
        let unknown = fluke.replacen("max_counts = 6000", "max_counts = 6000\nfudge = 1", 1);
        assert!(matches!(parse(&unknown), Err(ProfileError::Toml(_))));
        let wrong = fluke.replacen("kind = \"multimeter\"", "kind = \"scope\"", 1);
        assert!(matches!(parse(&wrong), Err(ProfileError::Kind(_))));
    }
}
