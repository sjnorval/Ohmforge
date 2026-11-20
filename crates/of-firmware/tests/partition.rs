//! Partition table parsing, the MD5 row and the boot-app choice.

use of_firmware::Partition;
use of_firmware::partition::{PartitionError, PartitionTable, SUBTYPE_FACTORY, SUBTYPE_OTA_0, TABLE_SIZE, TYPE_APP};

#[test]
fn single_app_table_round_trips_with_md5() {
    let t = PartitionTable::single_app();
    let bytes = t.to_bytes();
    assert_eq!(bytes.len(), TABLE_SIZE);
    assert_eq!(&bytes[..2], &[0xaa, 0x50]);
    // The digest row follows the three entries.
    assert_eq!(&bytes[96..98], &[0xeb, 0xeb]);
    let parsed = PartitionTable::parse(&bytes).expect("valid");
    assert_eq!(parsed, t);
    let app = parsed.boot_app().expect("factory");
    assert_eq!(
        (app.label.as_str(), app.offset, app.size),
        ("factory", 0x1_0000, 0x10_0000)
    );
}

#[test]
fn md5_mismatch_and_missing_tables_are_rejected() {
    let mut bytes = PartitionTable::single_app().to_bytes();
    bytes[4] ^= 1;
    assert_eq!(PartitionTable::parse(&bytes), Err(PartitionError::Md5));
    assert_eq!(
        PartitionTable::parse(&[0xff; 64]),
        Err(PartitionError::NoTable(0xff, 0xff))
    );
    assert_eq!(
        PartitionTable::parse(&[0xe9; 64]),
        Err(PartitionError::NoTable(0xe9, 0xe9))
    );
}

#[test]
fn ota_app_is_chosen_without_a_factory_app() {
    let app = |subtype, offset, label: &str| Partition {
        kind: TYPE_APP,
        subtype,
        offset,
        size: 0x10_0000,
        label: label.into(),
        flags: 0,
    };
    let t = PartitionTable {
        entries: vec![
            app(SUBTYPE_OTA_0 + 1, 0x11_0000, "ota_1"),
            app(SUBTYPE_OTA_0, 0x1_0000, "ota_0"),
        ],
        has_md5: true,
    };
    assert_eq!(
        PartitionTable::parse(&t.to_bytes())
            .expect("valid")
            .boot_app()
            .map(|p| p.label.as_str()),
        Some("ota_0")
    );
    let f = PartitionTable {
        entries: vec![app(SUBTYPE_FACTORY, 0x2_0000, "factory")],
        has_md5: false,
    };
    // Without the MD5 row (older tables) the entries still parse.
    let mut bytes = f.to_bytes();
    bytes[32..64].fill(0xff);
    let parsed = PartitionTable::parse(&bytes).expect("valid");
    assert!(!parsed.has_md5);
    assert_eq!(parsed.boot_app().map(|p| p.offset), Some(0x2_0000));
}
