//! Bill of materials derived from placed parts.
//!
//! For release 0.1 the BOM is built from the desktop demo boards' placements
//! ([`demos`]). Engine E6 (#50) later builds the same structure from circuit
//! file components.

use std::collections::BTreeMap;

use of_parts::{Offer, PartFile, Variant};

pub mod demos;

/// One placed part that should appear on the BOM.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BomPlacement {
    /// Reference designator, for example `R1`.
    pub refdes: String,
    /// Library part id, for example `generic.resistor`.
    pub part_id: String,
}

impl BomPlacement {
    /// Placement of `refdes` using library part `part_id`.
    pub fn new(refdes: impl Into<String>, part_id: impl Into<String>) -> Self {
        Self {
            refdes: refdes.into(),
            part_id: part_id.into(),
        }
    }
}

/// Errors while building a BOM.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum BomError {
    /// A placement names a part that is not in the library map.
    #[error("unknown part '{0}'")]
    UnknownPart(String),
    /// A part has no `[[variant]]` to buy.
    #[error("part '{0}' has no purchasable variant")]
    NoVariant(String),
}

/// One BOM line: a purchasable variant and how many are needed.
#[derive(Debug, Clone, PartialEq)]
pub struct BomLine {
    /// Reference designators, for example `R1, R2`.
    pub references: Vec<String>,
    /// Library part id.
    pub part_id: String,
    /// Manufacturer part number.
    pub mpn: String,
    /// Manufacturer.
    pub manufacturer: String,
    /// Quantity.
    pub quantity: u32,
    /// Chosen supplier key, if any.
    pub supplier: Option<String>,
    /// Supplier SKU, if any.
    pub sku: Option<String>,
    /// Product page URL, if any.
    pub url: Option<String>,
    /// Unit price in ZAR cents, if known.
    pub unit_price_zar_cents: Option<u64>,
}

/// A whole bill of materials.
#[derive(Debug, Clone, Default)]
pub struct Bom {
    /// Lines in display order (first-seen part order, refs sorted within a line).
    pub lines: Vec<BomLine>,
    /// Optional board title for the checklist (for example `dimmer`).
    pub title: Option<String>,
}

impl Bom {
    /// Build a BOM from `placements`, looking up parts by id in `parts`.
    ///
    /// Placements for the same part id and MPN are merged. The first
    /// `[[variant]]` of each part is used. The first offer with a price is
    /// preferred; otherwise the first offer.
    pub fn from_placements(parts: &BTreeMap<String, PartFile>, placements: &[BomPlacement]) -> Result<Self, BomError> {
        // Preserve first-seen order of (part_id, mpn).
        let mut order: Vec<(String, String)> = Vec::new();
        let mut groups: BTreeMap<(String, String), LineDraft> = BTreeMap::new();

        for p in placements {
            let part = parts
                .get(&p.part_id)
                .ok_or_else(|| BomError::UnknownPart(p.part_id.clone()))?;
            let variant = part
                .variants
                .first()
                .ok_or_else(|| BomError::NoVariant(p.part_id.clone()))?;
            let key = (p.part_id.clone(), variant.mpn.clone());
            if !groups.contains_key(&key) {
                order.push(key.clone());
                groups.insert(key.clone(), LineDraft::from_variant(&p.part_id, variant));
            }
            groups
                .get_mut(&key)
                .expect("just inserted")
                .references
                .push(p.refdes.clone());
        }

        let mut lines = Vec::with_capacity(order.len());
        for key in order {
            let mut draft = groups.remove(&key).expect("ordered key");
            draft.references.sort();
            draft.quantity = draft.references.len() as u32;
            lines.push(draft.into_line());
        }
        Ok(Self { lines, title: None })
    }

    /// Set the board title used by [`Self::checklist`].
    pub fn with_title(mut self, title: impl Into<String>) -> Self {
        self.title = Some(title.into());
        self
    }

    /// Total cost in ZAR cents for lines with a known price.
    pub fn total_zar_cents(&self) -> u64 {
        self.lines
            .iter()
            .filter_map(|l| l.unit_price_zar_cents.map(|p| p * u64::from(l.quantity)))
            .sum()
    }

    /// Format a ZAR amount from cents as `R12.34`.
    pub fn format_zar(cents: u64) -> String {
        format!("R{}.{:02}", cents / 100, cents % 100)
    }

    /// CSV export (RFC 4180 quoting) with supplier links and ZAR prices.
    pub fn to_csv(&self) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("References,Manufacturer,MPN,Quantity,Supplier,SKU,URL,UnitPriceZAR,ExtPriceZAR\n");
        for l in &self.lines {
            let unit = l
                .unit_price_zar_cents
                .map(|c| format!("{}.{:02}", c / 100, c % 100))
                .unwrap_or_default();
            let ext = l
                .unit_price_zar_cents
                .map(|c| {
                    let e = c * u64::from(l.quantity);
                    format!("{}.{:02}", e / 100, e % 100)
                })
                .unwrap_or_default();
            out += &format!(
                "{},{},{},{},{},{},{},{},{}\n",
                q(&l.references.join(", ")),
                q(&l.manufacturer),
                q(&l.mpn),
                l.quantity,
                q(l.supplier.as_deref().unwrap_or("")),
                q(l.sku.as_deref().unwrap_or("")),
                q(l.url.as_deref().unwrap_or("")),
                unit,
                ext
            );
        }
        let total_cents = self.total_zar_cents();
        let total = format!("{}.{:02}", total_cents / 100, total_cents % 100);
        out += &format!(
            "{},{},{},{},{},{},{},{},{}\n",
            q("TOTAL"),
            q(""),
            q(""),
            "",
            q(""),
            q(""),
            q(""),
            "",
            total
        );
        out
    }

    /// Markdown build checklist: buy list, then generic pre-power and first-power steps.
    pub fn checklist(&self) -> String {
        let title = self.title.as_deref().unwrap_or("board");
        let mut out = format!("# Build checklist: {title}\n\n");
        out.push_str("## Buy\n\n");
        for l in &self.lines {
            let refs = l.references.join(", ");
            let price = l
                .unit_price_zar_cents
                .map(|c| format!(" — {}", Self::format_zar(c * u64::from(l.quantity))))
                .unwrap_or_default();
            let link = l.url.as_ref().map(|u| format!(" ([order]({u}))")).unwrap_or_default();
            out.push_str(&format!(
                "- [ ] **{refs}** ×{}: {} {} (`{}`){price}{link}\n",
                l.quantity, l.manufacturer, l.mpn, l.part_id
            ));
        }
        out.push_str(&format!(
            "\n**Parts total (known prices):** {}\n\n",
            Self::format_zar(self.total_zar_cents())
        ));
        out.push_str("## Wire\n\n");
        out.push_str("- [ ] Place each part on the breadboard / PCB per the schematic references\n");
        out.push_str("- [ ] Wire power and ground before signal nets\n");
        out.push_str("- [ ] Double-check LED cathode (K) to ground and series resistor on the anode path\n\n");
        out.push_str("## Pre-power checks\n\n");
        out.push_str("- [ ] Supply set to 3.3 V (ESP32-C3 absolute max 3.6 V)\n");
        out.push_str("- [ ] No shorts between 3V3 and GND\n");
        out.push_str("- [ ] LED polarity correct; series resistor present\n\n");
        out.push_str("## First power\n\n");
        out.push_str("- [ ] 3V3 rail reads about 3.3 V\n");
        out.push_str("- [ ] Firmware boots (UART banner if attached)\n");
        out.push_str("- [ ] LED behaviour matches the demo (blink / dim / fade / button)\n");
        out
    }
}

struct LineDraft {
    part_id: String,
    mpn: String,
    manufacturer: String,
    references: Vec<String>,
    quantity: u32,
    offer: Option<Offer>,
}

impl LineDraft {
    fn from_variant(part_id: &str, variant: &Variant) -> Self {
        Self {
            part_id: part_id.to_owned(),
            mpn: variant.mpn.clone(),
            manufacturer: variant.manufacturer.clone(),
            references: Vec::new(),
            quantity: 0,
            offer: pick_offer(&variant.offers),
        }
    }

    fn into_line(self) -> BomLine {
        let (supplier, sku, url, price) = match self.offer {
            Some(o) => (Some(o.supplier), Some(o.sku), o.url, o.price_zar_cents),
            None => (None, None, None, None),
        };
        BomLine {
            references: self.references,
            part_id: self.part_id,
            mpn: self.mpn,
            manufacturer: self.manufacturer,
            quantity: self.quantity,
            supplier,
            sku,
            url,
            unit_price_zar_cents: price,
        }
    }
}

fn pick_offer(offers: &[Offer]) -> Option<Offer> {
    offers
        .iter()
        .find(|o| o.price_zar_cents.is_some())
        .or_else(|| offers.first())
        .cloned()
}

/// Load every `part.toml` under `dir` into an id → part map.
pub fn load_parts_map(dir: &std::path::Path) -> Result<BTreeMap<String, PartFile>, String> {
    let loaded = of_parts::load_dir(dir).map_err(|e| e.to_string())?;
    let mut map = BTreeMap::new();
    for (_path, part) in loaded {
        let id = part.part.id.clone();
        map.insert(id, part);
    }
    Ok(map)
}

#[cfg(test)]
mod tests {
    use super::*;
    use of_parts::parse_part;

    fn resistor_330() -> PartFile {
        parse_part(
            r#"
[part]
id = "generic.resistor"
name = "Chip resistor"
category = "passive/resistor"
model = "rust:resistor"
verification = "datasheet"
[[pin]]
number = "1"
name = "1"
[[pin]]
number = "2"
name = "2"
[[variant]]
label = "330 ohm"
mpn = "RC0603FR-07330RL"
manufacturer = "Yageo"
offers = [{ supplier = "lcsc", sku = "C22805", url = "https://www.lcsc.com/product-detail/C22805.html", price_zar_cents = 12 }]
[license]
data = "CC-BY-SA-4.0"
artwork = "CC-BY-SA-4.0"
"#,
        )
        .unwrap()
    }

    #[test]
    fn totals_csv_and_checklist() {
        let mut parts = BTreeMap::new();
        parts.insert("generic.resistor".into(), resistor_330());
        let bom = Bom::from_placements(
            &parts,
            &[
                BomPlacement::new("R1", "generic.resistor"),
                BomPlacement::new("R2", "generic.resistor"),
            ],
        )
        .unwrap()
        .with_title("test");
        assert_eq!(bom.total_zar_cents(), 24);
        let csv = bom.to_csv();
        assert!(csv.contains("\"R1, R2\""));
        assert!(csv.contains("RC0603FR-07330RL"));
        assert!(csv.contains("https://www.lcsc.com/product-detail/C22805.html"));
        assert!(csv.contains("0.12"));
        assert!(csv.contains("TOTAL"));
        let check = bom.checklist();
        assert!(check.contains("Build checklist: test"));
        assert!(check.contains("R1, R2"));
        assert!(check.contains("Pre-power"));
    }

    #[test]
    fn unknown_part_errors() {
        let parts = BTreeMap::new();
        let err = Bom::from_placements(&parts, &[BomPlacement::new("R1", "nope")]).unwrap_err();
        assert_eq!(err, BomError::UnknownPart("nope".into()));
    }
}
