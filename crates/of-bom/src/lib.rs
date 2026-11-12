//! Bill of materials derived from the design.

/// One BOM line: a purchasable variant and how many are needed.
#[derive(Debug, Clone, PartialEq)]
pub struct BomLine {
    /// Reference designators, for example `R1, R2`.
    pub references: Vec<String>,
    /// Manufacturer part number.
    pub mpn: String,
    /// Manufacturer.
    pub manufacturer: String,
    /// Quantity.
    pub quantity: u32,
    /// Unit price in ZAR cents, if known.
    pub unit_price_zar_cents: Option<u64>,
}

/// A whole bill of materials.
#[derive(Debug, Clone, Default)]
pub struct Bom {
    /// Lines in display order.
    pub lines: Vec<BomLine>,
}

impl Bom {
    /// Total cost in ZAR cents for lines with a known price.
    pub fn total_zar_cents(&self) -> u64 {
        self.lines
            .iter()
            .filter_map(|l| l.unit_price_zar_cents.map(|p| p * u64::from(l.quantity)))
            .sum()
    }

    /// Minimal CSV export (RFC 4180 quoting).
    pub fn to_csv(&self) -> String {
        let q = |s: &str| format!("\"{}\"", s.replace('"', "\"\""));
        let mut out = String::from("References,Manufacturer,MPN,Quantity,UnitPriceZAR\n");
        for l in &self.lines {
            let price = l
                .unit_price_zar_cents
                .map(|c| format!("{}.{:02}", c / 100, c % 100))
                .unwrap_or_default();
            out += &format!(
                "{},{},{},{},{}\n",
                q(&l.references.join(", ")),
                q(&l.manufacturer),
                q(&l.mpn),
                l.quantity,
                price
            );
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn totals_and_csv() {
        let bom = Bom {
            lines: vec![BomLine {
                references: vec!["R1".into(), "R2".into()],
                mpn: "RC0805FR-0710KL".into(),
                manufacturer: "Yageo".into(),
                quantity: 2,
                unit_price_zar_cents: Some(150),
            }],
        };
        assert_eq!(bom.total_zar_cents(), 300);
        assert!(bom.to_csv().contains("\"R1, R2\",\"Yageo\",\"RC0805FR-0710KL\",2,1.50"));
    }
}
