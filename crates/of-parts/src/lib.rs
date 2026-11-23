//! Parts library: the `part.toml` schema and a loader.
//!
//! Part data is licensed CC-BY-SA-4.0 (see parts/LICENSE.md).
//!
//! Besides metadata, electrical ratings and purchasable variants, a part
//! may list its pins (`[[pin]]`) and an analog model (`[analog]`): either a
//! primitive device (resistor, capacitor, LED, ...) or a SPICE subcircuit
//! from a library file next to `part.toml`, with min/typ/max/tolerance
//! parameters. `of-components` turns a placed part into solver devices.

use serde::Deserialize;
use std::collections::BTreeMap;

/// A min/typ/max electrical parameter with a unit.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Param {
    /// Minimum value.
    pub min: Option<f64>,
    /// Typical value.
    pub typ: Option<f64>,
    /// Maximum value.
    pub max: Option<f64>,
    /// SI unit, for example `V` or `A`.
    pub unit: String,
    /// Free-text note.
    pub note: Option<String>,
}

/// A supplier offer for a variant.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Offer {
    /// Supplier key, for example `lcsc`.
    pub supplier: String,
    /// Supplier SKU.
    pub sku: String,
    /// Product page URL, if known.
    #[serde(default)]
    pub url: Option<String>,
    /// Snapshot unit price in ZAR cents (dated; not live).
    #[serde(default)]
    pub price_zar_cents: Option<u64>,
}

/// A purchasable variant of a part.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Variant {
    /// Human label.
    pub label: String,
    /// Manufacturer part number.
    pub mpn: String,
    /// Manufacturer.
    pub manufacturer: String,
    /// Footprint reference (name only).
    pub footprint: Option<String>,
    /// Supplier offers.
    #[serde(default)]
    pub offers: Vec<Offer>,
}

/// Core metadata of a part.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PartMeta {
    /// Unique id, for example `bosch.bme280`.
    pub id: String,
    /// Display name.
    pub name: String,
    /// Category path.
    pub category: String,
    /// Datasheet URL.
    pub datasheet: Option<String>,
    /// Behaviour model reference (`rust:`, `wasm:` or `spice:`).
    pub model: String,
    /// `datasheet`, `bench-measured` or `community`.
    pub verification: String,
}

/// Licence of the part data and artwork.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PartLicense {
    /// Licence of the data, normally `CC-BY-SA-4.0`.
    pub data: String,
    /// Licence of symbols and breadboard art.
    pub artwork: String,
}

/// A pin of the part.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct Pin {
    /// Pin number on the package, for example `1`.
    pub number: String,
    /// Pin name, for example `VDD`.
    pub name: String,
    /// `power`, `input`, `output`, `io` or `passive`.
    pub kind: Option<String>,
}

/// Which value of a toleranced parameter to use.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub enum Corner {
    /// The typical value.
    #[default]
    Typical,
    /// The minimum (or `typ * (1 - tol)`).
    Min,
    /// The maximum (or `typ * (1 + tol)`).
    Max,
    /// A deviation in -1..=1 from typical towards min (negative) or max
    /// (positive), for Monte Carlo and sweeps.
    Deviation(f64),
}

/// A model parameter with optional spread.
#[derive(Debug, Clone, Deserialize, PartialEq, Default)]
#[serde(deny_unknown_fields)]
pub struct ParamSpec {
    /// Minimum value.
    pub min: Option<f64>,
    /// Typical value.
    pub typ: Option<f64>,
    /// Maximum value.
    pub max: Option<f64>,
    /// Relative tolerance, e.g. 0.01 for 1 % (used when min/max are absent).
    pub tol: Option<f64>,
    /// SI unit.
    pub unit: Option<String>,
    /// Free-text note (source of the value, conditions).
    pub note: Option<String>,
}

impl ParamSpec {
    /// A bare value.
    pub fn value(v: f64) -> Self {
        Self {
            typ: Some(v),
            ..Self::default()
        }
    }

    /// The typical value: `typ`, else the middle of `min` and `max`.
    pub fn typical(&self) -> Option<f64> {
        self.typ.or(match (self.min, self.max) {
            (Some(a), Some(b)) => Some(0.5 * (a + b)),
            (a, b) => a.or(b),
        })
    }

    fn low(&self) -> Option<f64> {
        self.min.or_else(|| Some(self.typical()? * (1.0 - self.tol?)))
    }

    fn high(&self) -> Option<f64> {
        self.max.or_else(|| Some(self.typical()? * (1.0 + self.tol?)))
    }

    /// The value at a corner (the typical value when the spread is unknown).
    pub fn at(&self, corner: Corner) -> Option<f64> {
        let typ = self.typical()?;
        Some(match corner {
            Corner::Typical => typ,
            Corner::Min => self.low().unwrap_or(typ),
            Corner::Max => self.high().unwrap_or(typ),
            Corner::Deviation(k) => {
                let k = k.clamp(-1.0, 1.0);
                if k < 0.0 {
                    typ + k * (typ - self.low().unwrap_or(typ))
                } else {
                    typ + k * (self.high().unwrap_or(typ) - typ)
                }
            }
        })
    }

    /// Relative tolerance: `tol`, else derived from min/max around typ.
    pub fn tolerance(&self) -> Option<f64> {
        self.tol.or_else(|| {
            let typ = self.typical()?;
            let spread = (self.max? - typ).abs().max((typ - self.min?).abs());
            (typ != 0.0).then(|| spread / typ.abs())
        })
    }
}

/// A model parameter: a bare number or a [`ParamSpec`] table.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(untagged)]
pub enum ModelParam {
    /// A plain value.
    Value(f64),
    /// Value with spread and unit.
    Spec(ParamSpec),
}

impl ModelParam {
    /// As a [`ParamSpec`].
    pub fn spec(&self) -> ParamSpec {
        match self {
            Self::Value(v) => ParamSpec::value(*v),
            Self::Spec(s) => s.clone(),
        }
    }
}

/// Analog device kinds a part can map to.
pub const ANALOG_KINDS: &[&str] = &[
    "resistor",
    "capacitor",
    "inductor",
    "diode",
    "led",
    "npn",
    "pnp",
    "load",
    "switch",
    "potentiometer",
    "subckt",
];

/// How the part is simulated by the analog solver.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnalogModel {
    /// One of [`ANALOG_KINDS`].
    pub kind: String,
    /// Part pins (numbers or names) in device terminal order, e.g.
    /// anode then cathode, or the subcircuit's port order.
    pub pins: Vec<String>,
    /// Model parameters. Primitive kinds use `resistance`, `capacitance`
    /// (with `esr`, `esl`), `inductance` (with `rser`), SPICE diode/BJT model
    /// names (`is`, `n`, `rs`, `bf`, ...), LED extras (`vf`, `if`,
    /// `wavelength`, `eqe`), `tc1`/`tc2`, `voltage` and `current` for a
    /// `load`, `ron` and `roff` for a `switch` (state from the placement
    /// parameter `closed`), or `resistance` (end to end) for a
    /// `potentiometer` (pins: end, wiper, end; wiper from the placement
    /// parameter `position`). For `subckt` they are passed as subcircuit
    /// parameters.
    #[serde(default)]
    pub params: BTreeMap<String, ModelParam>,
    /// SPICE library file, relative to the directory of `part.toml`.
    pub file: Option<String>,
    /// Subcircuit name in `file`.
    pub subckt: Option<String>,
    /// Notes on the model's origin and limits.
    pub note: Option<String>,
}

/// A whole `part.toml` file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PartFile {
    /// Metadata.
    pub part: PartMeta,
    /// Pins.
    #[serde(default, rename = "pin")]
    pub pins: Vec<Pin>,
    /// Electrical parameters (free-form keys).
    #[serde(default)]
    pub electrical: BTreeMap<String, toml::Value>,
    /// Analog behaviour, if the part takes part in the analog solve.
    pub analog: Option<AnalogModel>,
    /// Purchasable variants.
    #[serde(default, rename = "variant")]
    pub variants: Vec<Variant>,
    /// Licence block (required).
    pub license: PartLicense,
}

impl PartFile {
    /// The pin with this number or (case-insensitive) name.
    pub fn pin(&self, key: &str) -> Option<&Pin> {
        self.pins
            .iter()
            .find(|p| p.number == key)
            .or_else(|| self.pins.iter().find(|p| p.name.eq_ignore_ascii_case(key)))
    }

    /// Consistency problems (empty when the part is valid).
    pub fn check(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut seen = std::collections::BTreeSet::new();
        for p in &self.pins {
            if !seen.insert(p.number.as_str()) {
                out.push(format!("pin number {} is used twice", p.number));
            }
        }
        if let Some(a) = &self.analog {
            if !ANALOG_KINDS.contains(&a.kind.as_str()) {
                out.push(format!("unknown analog kind '{}'", a.kind));
            }
            let want = match a.kind.as_str() {
                "npn" | "pnp" | "potentiometer" => Some(3),
                "subckt" => None,
                _ => Some(2),
            };
            if let Some(n) = want.filter(|&n| n != a.pins.len()) {
                out.push(format!("analog kind '{}' needs {n} pins, got {}", a.kind, a.pins.len()));
            }
            for p in &a.pins {
                if self.pin(p).is_none() {
                    out.push(format!("analog pin '{p}' is not a pin of the part"));
                }
            }
            if a.kind == "subckt" && (a.file.is_none() || a.subckt.is_none()) {
                out.push("analog kind 'subckt' needs 'file' and 'subckt'".into());
            }
            for (k, v) in &a.params {
                if v.spec().typical().is_none() {
                    out.push(format!("analog parameter '{k}' has no value"));
                }
            }
        }
        out
    }
}

/// Loader errors.
#[derive(Debug, thiserror::Error)]
pub enum PartError {
    /// TOML syntax or schema error.
    #[error("invalid part.toml: {0}")]
    Parse(#[from] toml::de::Error),
}

/// Parse a `part.toml` document.
pub fn parse_part(src: &str) -> Result<PartFile, PartError> {
    Ok(toml::from_str(src)?)
}

/// Read every `part.toml` below `dir` (sorted by path), with the
/// directory each one lives in.
pub fn load_dir(dir: &std::path::Path) -> Result<Vec<(std::path::PathBuf, PartFile)>, LoadError> {
    let mut found = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(d) = stack.pop() {
        let entries = std::fs::read_dir(&d).map_err(|e| LoadError::Io(d.display().to_string(), e.to_string()))?;
        for e in entries {
            let p = e
                .map_err(|e| LoadError::Io(d.display().to_string(), e.to_string()))?
                .path();
            if p.is_dir() {
                stack.push(p);
            } else if p.file_name().is_some_and(|n| n == "part.toml") {
                found.push(p);
            }
        }
    }
    found.sort();
    found
        .into_iter()
        .map(|p| {
            let text =
                std::fs::read_to_string(&p).map_err(|e| LoadError::Io(p.display().to_string(), e.to_string()))?;
            let part = parse_part(&text).map_err(|e| LoadError::Part(p.display().to_string(), e.to_string()))?;
            Ok((p.parent().map(|d| d.to_path_buf()).unwrap_or_default(), part))
        })
        .collect()
}

/// Errors reading a parts directory.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// A file or directory could not be read.
    #[error("{0}: {1}")]
    Io(String, String),
    /// A `part.toml` did not parse.
    #[error("{0}: {1}")]
    Part(String, String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_library_part_parses_and_checks() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts");
        let parts = load_dir(&dir).unwrap();
        assert!(parts.len() >= 7, "found {}", parts.len());
        let mut ids = std::collections::BTreeSet::new();
        for (d, p) in &parts {
            assert!(ids.insert(p.part.id.clone()), "duplicate id {}", p.part.id);
            assert_eq!(p.license.data, "CC-BY-SA-4.0", "{}", p.part.id);
            assert!(p.check().is_empty(), "{}: {:?}", p.part.id, p.check());
            if let Some(f) = p.analog.as_ref().and_then(|a| a.file.as_ref()) {
                assert!(d.join(f).is_file(), "{}: missing {f}", p.part.id);
            }
        }
    }

    #[test]
    fn corners_and_tolerances() {
        let s = ParamSpec {
            typ: Some(100.0),
            tol: Some(0.05),
            ..ParamSpec::default()
        };
        assert_eq!(s.at(Corner::Typical), Some(100.0));
        assert!((s.at(Corner::Min).unwrap() - 95.0).abs() < 1e-12);
        assert!((s.at(Corner::Max).unwrap() - 105.0).abs() < 1e-12);
        assert!((s.at(Corner::Deviation(-0.5)).unwrap() - 97.5).abs() < 1e-12);
        let m = ParamSpec {
            min: Some(0.9),
            max: Some(1.5),
            typ: Some(1.1),
            ..ParamSpec::default()
        };
        assert_eq!(m.at(Corner::Max), Some(1.5));
        assert!((m.tolerance().unwrap() - 0.4 / 1.1).abs() < 1e-12);
        let bare: AnalogModel = toml::from_str(
            "kind = 'resistor'\npins = ['1', '2']\nparams = { resistance = 330.0, tc1 = { typ = 1e-4 } }",
        )
        .unwrap();
        assert_eq!(bare.params["resistance"].spec().typical(), Some(330.0));
        assert_eq!(bare.params["tc1"].spec().typical(), Some(1e-4));
    }

    #[test]
    fn check_reports_bad_analog_sections() {
        let src = include_str!("../../../parts/sensors/bosch-bme280/part.toml");
        let mut p = parse_part(src).unwrap();
        let a = p.analog.as_mut().unwrap();
        a.pins = vec!["VDD".into(), "nope".into()];
        a.kind = "flux-capacitor".into();
        let problems = p.check();
        assert!(problems.iter().any(|m| m.contains("nope")), "{problems:?}");
        assert!(problems.iter().any(|m| m.contains("flux-capacitor")), "{problems:?}");
    }

    #[test]
    fn example_part_parses() {
        let src = include_str!("../../../parts/sensors/bosch-bme280/part.toml");
        let p = parse_part(src).unwrap();
        assert_eq!(p.part.id, "bosch.bme280");
        assert_eq!(p.license.data, "CC-BY-SA-4.0");
        assert!(!p.variants.is_empty());
    }
}
