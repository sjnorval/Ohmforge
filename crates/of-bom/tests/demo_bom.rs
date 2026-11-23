//! BOM v1 acceptance: dimmer CSV lists U1, RV1, R2, D1 with MPNs and ZAR,
//! and excludes the bench PSU and instruments. The same path covers the
//! other demo boards.

use of_bom::{Bom, demos, load_parts_map};

fn parts_dir() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../parts")
}

#[test]
fn dimmer_csv_lists_orderable_parts_and_excludes_psu() {
    let parts = load_parts_map(&parts_dir()).expect("parts");
    let bom = Bom::from_placements(&parts, &demos::dimmer())
        .expect("dimmer bom")
        .with_title("dimmer");

    let refs: Vec<String> = bom.lines.iter().flat_map(|l| l.references.iter().cloned()).collect();
    // Schematic order on the dimmer board (R2 is the series LED resistor).
    assert_eq!(refs, vec!["U1", "RV1", "R2", "D1"]);

    for forbidden in ["PSU", "PSU1", "VM1", "AM1", "SCOPE"] {
        assert!(
            !refs.iter().any(|r| r.eq_ignore_ascii_case(forbidden)),
            "{forbidden} must not appear on the BOM"
        );
    }

    let by_ref: std::collections::BTreeMap<&str, &of_bom::BomLine> = bom
        .lines
        .iter()
        .flat_map(|l| l.references.iter().map(move |r| (r.as_str(), l)))
        .collect();
    assert_eq!(by_ref["U1"].mpn, "ESP32-C3-MINI-1-N4");
    assert_eq!(by_ref["RV1"].mpn, "PTV09A-4020F-B103");
    assert_eq!(by_ref["R2"].mpn, "RC0603FR-07330RL");
    assert_eq!(by_ref["D1"].mpn, "APT1608EC");

    assert!(bom.total_zar_cents() > 0);
    for line in &bom.lines {
        assert!(
            line.unit_price_zar_cents.is_some(),
            "{} needs a ZAR price snapshot",
            line.references.join(",")
        );
        assert!(line.url.as_ref().is_some_and(|u| u.starts_with("http")));
    }

    let csv = bom.to_csv();
    assert!(csv.contains("ESP32-C3-MINI-1-N4"));
    assert!(csv.contains("PTV09A-4020F-B103"));
    assert!(csv.contains("RC0603FR-07330RL"));
    assert!(csv.contains("APT1608EC"));
    assert!(csv.contains("TOTAL"));
    assert!(!csv.to_ascii_lowercase().contains("psu"));

    let check = bom.checklist();
    assert!(check.contains("Build checklist: dimmer"));
    assert!(check.contains("U1"));
    assert!(check.contains("Pre-power"));
}

#[test]
fn other_demo_boards_build() {
    let parts = load_parts_map(&parts_dir()).expect("parts");
    for name in demos::names() {
        let placements = demos::by_name(name).expect(name);
        let bom = Bom::from_placements(&parts, &placements)
            .unwrap_or_else(|e| panic!("{name}: {e}"))
            .with_title(*name);
        assert!(
            bom.lines.iter().any(|l| l.references.iter().any(|r| r == "U1")),
            "{name} must include U1"
        );
        assert!(
            !bom.lines
                .iter()
                .flat_map(|l| &l.references)
                .any(|r| r.eq_ignore_ascii_case("PSU")),
            "{name} must exclude PSU"
        );
        assert!(bom.total_zar_cents() > 0, "{name} needs priced lines");
        assert!(!bom.to_csv().is_empty());
        assert!(bom.checklist().contains("Buy"));
    }
}
