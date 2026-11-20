//! ESP partition tables (the table ESP-IDF's `gen_esp32part.py` writes at
//! flash offset `0x8000`, read by the second-stage bootloader).
//!
//! The table is a sequence of 32-byte entries: magic `0xAA 0x50`, type,
//! subtype, offset (u32), size (u32), a 16-byte NUL-padded label and flags
//! (u32), all little-endian. An optional digest row (magic `0xEB 0xEB`, 14
//! bytes of `0xFF`, then the MD5 of all entries before it) follows the
//! entries; a row of `0xFF` ends the table. At most 95 entries fit the
//! 3 KiB table area.

use thiserror::Error;

use crate::md5::md5;

/// Where the bootloader looks for the table.
pub const TABLE_OFFSET: u32 = 0x8000;
/// Size of the table area.
pub const TABLE_SIZE: usize = 0xc00;
const ENTRY: usize = 32;
const MAGIC: [u8; 2] = [0xaa, 0x50];
const MD5_MAGIC: [u8; 2] = [0xeb, 0xeb];

/// Partition type `app`.
pub const TYPE_APP: u8 = 0x00;
/// Partition type `data`.
pub const TYPE_DATA: u8 = 0x01;
/// App subtype `factory`.
pub const SUBTYPE_FACTORY: u8 = 0x00;
/// App subtype `ota_0` (`ota_n` is `0x10 + n`).
pub const SUBTYPE_OTA_0: u8 = 0x10;
/// App subtype `test`.
pub const SUBTYPE_TEST: u8 = 0x20;
/// Data subtype `ota` (OTA selection data).
pub const SUBTYPE_OTADATA: u8 = 0x00;
/// Data subtype `phy`.
pub const SUBTYPE_PHY: u8 = 0x01;
/// Data subtype `nvs`.
pub const SUBTYPE_NVS: u8 = 0x02;

/// Why a partition table was rejected.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PartitionError {
    /// The first row is neither an entry nor the end marker.
    #[error("no partition table (first bytes {0:02x} {1:02x})")]
    NoTable(u8, u8),
    /// A row has an unknown magic.
    #[error("bad partition table row {0}")]
    BadRow(usize),
    /// The MD5 row does not match the entries.
    #[error("partition table MD5 mismatch")]
    Md5,
    /// The table runs past the bytes given or the 3 KiB area.
    #[error("partition table is not terminated")]
    Unterminated,
}

/// One partition.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Partition {
    /// Type ([`TYPE_APP`], [`TYPE_DATA`], or custom 0x40..0xFE).
    pub kind: u8,
    /// Subtype.
    pub subtype: u8,
    /// Flash offset.
    pub offset: u32,
    /// Size in bytes.
    pub size: u32,
    /// Label (up to 16 bytes).
    pub label: String,
    /// Flags (bit 0: encrypted, bit 1: read-only).
    pub flags: u32,
}

impl Partition {
    /// Whether this is an app partition.
    pub fn is_app(&self) -> bool {
        self.kind == TYPE_APP
    }
}

/// A parsed table.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PartitionTable {
    /// Entries in table order.
    pub entries: Vec<Partition>,
    /// Whether an MD5 row was present (and matched).
    pub has_md5: bool,
}

impl PartitionTable {
    /// Parse a table from the start of `bytes` (normally flash from
    /// [`TABLE_OFFSET`]).
    pub fn parse(bytes: &[u8]) -> Result<Self, PartitionError> {
        let mut t = Self::default();
        for (i, row) in bytes.chunks(ENTRY).take(TABLE_SIZE / ENTRY).enumerate() {
            if row.len() < ENTRY {
                break;
            }
            let u32_at = |o: usize| u32::from_le_bytes([row[o], row[o + 1], row[o + 2], row[o + 3]]);
            match [row[0], row[1]] {
                MAGIC => {
                    let label = &row[12..28];
                    let end = label.iter().position(|&b| b == 0).unwrap_or(16);
                    t.entries.push(Partition {
                        kind: row[2],
                        subtype: row[3],
                        offset: u32_at(4),
                        size: u32_at(8),
                        label: String::from_utf8_lossy(&label[..end]).into_owned(),
                        flags: u32_at(28),
                    });
                }
                MD5_MAGIC => {
                    if md5(&bytes[..i * ENTRY]) != row[16..32] {
                        return Err(PartitionError::Md5);
                    }
                    t.has_md5 = true;
                }
                [0xff, 0xff] if i == 0 => return Err(PartitionError::NoTable(0xff, 0xff)),
                [0xff, 0xff] => return Ok(t),
                [a, b] if i == 0 => return Err(PartitionError::NoTable(a, b)),
                _ => return Err(PartitionError::BadRow(i)),
            }
        }
        Err(PartitionError::Unterminated)
    }

    /// Serialise with an MD5 row and the end marker (`0xFF` padding to a
    /// full 3 KiB table area).
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(TABLE_SIZE);
        for p in &self.entries {
            out.extend_from_slice(&MAGIC);
            out.extend_from_slice(&[p.kind, p.subtype]);
            out.extend_from_slice(&p.offset.to_le_bytes());
            out.extend_from_slice(&p.size.to_le_bytes());
            let mut label = [0u8; 16];
            let n = p.label.len().min(16);
            label[..n].copy_from_slice(&p.label.as_bytes()[..n]);
            out.extend_from_slice(&label);
            out.extend_from_slice(&p.flags.to_le_bytes());
        }
        let digest = md5(&out);
        out.extend_from_slice(&MD5_MAGIC);
        out.extend_from_slice(&[0xff; 14]);
        out.extend_from_slice(&digest);
        out.resize(TABLE_SIZE, 0xff);
        out
    }

    /// The app partition the bootloader would start without OTA data:
    /// `factory`, else the lowest `ota_n`, else `test`. (Choosing by the
    /// `otadata` partition is not implemented.)
    pub fn boot_app(&self) -> Option<&Partition> {
        let app = |sub: &dyn Fn(u8) -> bool| self.entries.iter().find(|p| p.is_app() && sub(p.subtype));
        app(&|s| s == SUBTYPE_FACTORY)
            .or_else(|| {
                self.entries
                    .iter()
                    .filter(|p| p.is_app() && (SUBTYPE_OTA_0..SUBTYPE_OTA_0 + 16).contains(&p.subtype))
                    .min_by_key(|p| p.subtype)
            })
            .or_else(|| app(&|s| s == SUBTYPE_TEST))
    }

    /// ESP-IDF's default single-app table (`partitions_singleapp.csv`):
    /// nvs at 0x9000 (24 KiB), phy_init at 0xf000 (4 KiB), factory app at
    /// 0x10000 (1 MiB).
    pub fn single_app() -> Self {
        let p = |kind, subtype, offset, size, label: &str| Partition {
            kind,
            subtype,
            offset,
            size,
            label: label.into(),
            flags: 0,
        };
        Self {
            entries: vec![
                p(TYPE_DATA, SUBTYPE_NVS, 0x9000, 0x6000, "nvs"),
                p(TYPE_DATA, SUBTYPE_PHY, 0xf000, 0x1000, "phy_init"),
                p(TYPE_APP, SUBTYPE_FACTORY, 0x1_0000, 0x10_0000, "factory"),
            ],
            has_md5: true,
        }
    }
}
