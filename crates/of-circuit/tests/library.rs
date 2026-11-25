//! Checks against the parts library and firmware hashes (feature `engine`).
#![cfg(feature = "engine")]

use std::collections::BTreeMap;

use of_circuit::library::{PartsIndex, Resolver, check_firmware, check_parts, normalize, sha256_hex, update_hashes};
use of_circuit::{parse, parse_unchecked, save};

fn library() -> PartsIndex {
    let parts = [
        include_str!("../../../parts/passives/resistor/part.toml"),
        include_str!("../../../parts/leds/red-0603/part.toml"),
        include_str!("../../../parts/mcu/esp32-c3/part.toml"),
    ];
    PartsIndex::new(parts.iter().map(|s| of_parts::parse_part(s).unwrap()))
}

/// In-memory firmware files.
struct Mem(BTreeMap<String, Vec<u8>>);

impl Resolver for Mem {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.0
            .get(path)
            .cloned()
            .ok_or_else(|| format!("no such file '{path}'"))
    }
}

const EMPTY_SHA: &str = "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855";

fn circuit(pins: &str, sha: &str) -> String {
    format!(
        "format = 1\n[circuit]\nname = \"x\"\n\
[[component]]\nref = \"U1\"\npart = \"espressif.esp32-c3\"\n\
[component.firmware]\napp = \"fw/blinky.elf\"\napp_sha256 = \"{sha}\"\nboot = \"direct\"\n\
[[component]]\nref = \"D1\"\npart = \"generic.led-red-0603\"\n\
[[component]]\nref = \"R1\"\npart = \"generic.resistor\"\n\
[[net]]\nname = \"led\"\npins = [\"U1.GPIO8\", \"R1.1\"]\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = [{pins}]\n"
    )
}

#[test]
fn sha256_hex_is_lower_case() {
    assert_eq!(sha256_hex(b""), EMPTY_SHA);
}

#[test]
fn known_parts_and_pins_pass() {
    let s = circuit("\"U1.GND\", \"D1.k\", \"r1.2\"", EMPTY_SHA);
    let f = parse(&s).unwrap();
    assert!(check_parts(&s, &f, &library()).is_empty());
}

#[test]
fn unknown_part_points_at_the_part_id() {
    let s = "format = 1\n[circuit]\nname = \"x\"\n[[component]]\nref = \"Q1\"\npart = \"acme.flux\"\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = [\"Q1.1\"]\n";
    let f = parse(s).unwrap();
    let errs = check_parts(s, &f, &library());
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!((errs[0].line, errs[0].column), (6, 8));
    assert!(errs[0].message.contains("unknown part 'acme.flux'"), "{}", errs[0]);
}

#[test]
fn unknown_pin_points_at_the_net_pin() {
    let s = circuit("\"U1.GND\", \"D1.X\"", EMPTY_SHA);
    let f = parse(&s).unwrap();
    let errs = check_parts(&s, &f, &library());
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!((errs[0].line, errs[0].column), (23, 19));
    assert!(
        errs[0].message.contains("D1.X") && errs[0].message.contains("has no pin 'X'"),
        "{}",
        errs[0]
    );
    assert!(errs[0].message.contains("A, K"), "{}", errs[0]);
}

#[test]
fn pin_numbers_are_not_pin_names() {
    // Pin number 1 of the LED is K; files use names only.
    let s = circuit("\"U1.GND\", \"D1.1\"", EMPTY_SHA);
    let f = parse(&s).unwrap();
    assert_eq!(check_parts(&s, &f, &library()).len(), 1);
}

#[test]
fn schematic_pin_must_be_a_part_pin() {
    let s = "format = 1\n[circuit]\nname = \"x\"\n[[component]]\nref = \"R1\"\npart = \"generic.resistor\"\n\
[component.schematic]\nbody = [0, 0, 1, 1]\npins = [{ pin = \"9\", at = [0, 0], dir = \"W\", stub = true }]\n\
[[net]]\nname = \"gnd\"\nkind = \"ground\"\npins = []\n";
    let f = parse(s).unwrap();
    let errs = check_parts(s, &f, &library());
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!((errs[0].line, errs[0].column), (9, 17));
}

#[test]
fn normalize_writes_back_the_canonical_case() {
    let s = circuit("\"u1.gnd\", \"d1.k\", \"r1.2\"", EMPTY_SHA);
    let mut f = parse(&s).unwrap();
    normalize(&mut f, Some(&library()));
    assert!(
        save(&f).contains("pins = [\"U1.GND\", \"D1.K\", \"R1.2\"]"),
        "{}",
        save(&f)
    );
    // Without the library only the ref case changes.
    let mut f = parse(&s).unwrap();
    normalize(&mut f, None);
    assert!(
        save(&f).contains("pins = [\"U1.gnd\", \"D1.k\", \"R1.2\"]"),
        "{}",
        save(&f)
    );
}

#[test]
fn firmware_hash_must_match() {
    let s = circuit("\"U1.GND\"", EMPTY_SHA);
    let f = parse(&s).unwrap();
    let ok = Mem(BTreeMap::from([("fw/blinky.elf".to_owned(), Vec::new())]));
    assert!(check_firmware(&s, &f, &ok).is_empty());

    let changed = Mem(BTreeMap::from([("fw/blinky.elf".to_owned(), b"abc".to_vec())]));
    let errs = check_firmware(&s, &f, &changed);
    assert_eq!(errs.len(), 1, "{errs:?}");
    assert_eq!((errs[0].line, errs[0].column), (9, 14));
    let new = sha256_hex(b"abc");
    assert!(
        errs[0].message.contains(&new) && errs[0].message.contains("--update-hashes"),
        "{}",
        errs[0]
    );

    let missing = Mem(BTreeMap::new());
    let errs = check_firmware(&s, &f, &missing);
    assert_eq!((errs[0].line, errs[0].column), (8, 7));
    assert!(errs[0].message.contains("no such file"), "{}", errs[0]);
}

#[test]
fn update_hashes_rewrites_stale_and_placeholder_hashes() {
    let s = circuit("\"U1.GND\"", "<64 hex>");
    assert!(parse(&s).is_err(), "a placeholder hash is not valid");
    let mut f = parse_unchecked(&s).unwrap();
    let fw = Mem(BTreeMap::from([("fw/blinky.elf".to_owned(), b"abc".to_vec())]));
    assert_eq!(update_hashes(&s, &mut f, &fw).unwrap(), 1);
    let text = save(&f);
    assert!(
        text.contains(&format!("app_sha256 = \"{}\"", sha256_hex(b"abc"))),
        "{text}"
    );
    let again = parse(&text).unwrap();
    assert!(check_firmware(&text, &again, &fw).is_empty());
    let mut again = again;
    assert_eq!(update_hashes(&text, &mut again, &fw).unwrap(), 0);
}
