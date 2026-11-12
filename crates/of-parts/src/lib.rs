//! Parts library: the `part.toml` schema and a loader.
//!
//! Part data is licensed CC-BY-4.0 (see parts/LICENSE.md).

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
    /// Licence of the data, normally `CC-BY-4.0`.
    pub data: String,
    /// Licence of symbols and breadboard art.
    pub artwork: String,
}

/// A whole `part.toml` file.
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct PartFile {
    /// Metadata.
    pub part: PartMeta,
    /// Electrical parameters (free-form keys).
    #[serde(default)]
    pub electrical: BTreeMap<String, toml::Value>,
    /// Purchasable variants.
    #[serde(default, rename = "variant")]
    pub variants: Vec<Variant>,
    /// Licence block (required).
    pub license: PartLicense,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn example_part_parses() {
        let src = include_str!("../../../parts/sensors/bosch-bme280/part.toml");
        let p = parse_part(src).unwrap();
        assert_eq!(p.part.id, "bosch.bme280");
        assert_eq!(p.license.data, "CC-BY-4.0");
        assert!(!p.variants.is_empty());
    }
}
