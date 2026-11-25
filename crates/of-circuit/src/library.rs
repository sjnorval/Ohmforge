//! Checks against the parts library and firmware files (feature `engine`).
//!
//! File access goes through [`Resolver`], so these checks also run in wasm32
//! with an embedded resolver; [`crate::fs::FsResolver`] reads from disk.

use std::collections::BTreeMap;

use of_parts::PartFile;
use toml::Spanned;

use crate::validate::{RefTarget, ref_table, split_pin};
use crate::{CircuitError, CircuitFile};

/// Library parts by id.
#[derive(Debug, Clone, Default)]
pub struct PartsIndex {
    parts: BTreeMap<String, PartFile>,
}

impl PartsIndex {
    /// Index parts by `[part] id`; the first part with an id wins.
    pub fn new(parts: impl IntoIterator<Item = PartFile>) -> Self {
        let mut map = BTreeMap::new();
        for p in parts {
            map.entry(p.part.id.clone()).or_insert(p);
        }
        Self { parts: map }
    }

    /// The part with this id.
    pub fn get(&self, id: &str) -> Option<&PartFile> {
        self.parts.get(id)
    }

    /// Number of parts.
    pub fn len(&self) -> usize {
        self.parts.len()
    }

    /// True when there are no parts.
    pub fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }
}

/// Reads files a circuit refers to (firmware, partition tables).
pub trait Resolver {
    /// Bytes of `path` as written in the file (relative to the circuit file,
    /// `/` separated). The error text names the file.
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    of_firmware::sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The part pin with this name (case-insensitive). Pin numbers do not match:
/// circuit files use names.
fn pin_named<'a>(part: &'a PartFile, name: &str) -> Option<&'a str> {
    part.pins
        .iter()
        .find(|p| p.name.eq_ignore_ascii_case(name))
        .map(|p| p.name.as_str())
}

fn pin_list(part: &PartFile) -> String {
    let mut names: Vec<&str> = part.pins.iter().map(|p| p.name.as_str()).collect();
    names.sort_unstable();
    names.join(", ")
}

/// Problems against the parts library, sorted by position: unknown part ids,
/// and net or schematic pins that the part does not have.
pub fn check_parts(src: &str, file: &CircuitFile, lib: &PartsIndex) -> Vec<CircuitError> {
    let mut out = Vec::new();
    for c in &file.components {
        let id = c.part_id();
        match lib.get(id) {
            None => out.push(CircuitError::at(
                src,
                c.part.span(),
                format!("{}: unknown part '{id}' (not in the parts library)", c.reference()),
            )),
            Some(part) => {
                for sp in c.schematic.iter().flat_map(|s| &s.pins) {
                    let p = sp.pin.get_ref();
                    if pin_named(part, p).is_none() {
                        out.push(CircuitError::at(
                            src,
                            sp.pin.span(),
                            format!(
                                "{}: part '{id}' has no pin '{p}' (pins: {})",
                                c.reference(),
                                pin_list(part)
                            ),
                        ));
                    }
                }
            }
        }
    }
    let refs = ref_table(file);
    for net in &file.nets {
        for pin in &net.pins {
            let text = pin.get_ref();
            let Some((r, p)) = split_pin(text) else { continue };
            let Some(&(RefTarget::Component(i), _)) = refs.get(&r.to_lowercase()) else {
                continue;
            };
            let c = &file.components[i];
            let Some(part) = lib.get(c.part_id()) else { continue };
            if pin_named(part, p).is_none() {
                out.push(CircuitError::at(
                    src,
                    pin.span(),
                    format!(
                        "'{text}': part '{}' has no pin '{p}' (pins: {})",
                        c.part_id(),
                        pin_list(part)
                    ),
                ));
            }
        }
    }
    out.sort_by_key(|e| (e.line, e.column));
    out
}

/// Write back the canonical case: refs in net pins as declared, pin names as
/// in `part.toml` (when `lib` is given) or in the symbol's pin list.
pub fn normalize(file: &mut CircuitFile, lib: Option<&PartsIndex>) {
    let canon: BTreeMap<String, (String, Option<String>)> = {
        let refs = ref_table(file);
        let mut m = BTreeMap::new();
        for (lower, (target, written)) in refs {
            let part = match target {
                RefTarget::Component(i) => Some(file.components[i].part_id().to_owned()),
                RefTarget::Symbol(_) => None,
            };
            m.insert(lower, (written.to_owned(), part));
        }
        m
    };
    let symbol_pins: BTreeMap<String, Vec<String>> = file
        .symbols
        .iter()
        .map(|y| {
            let pins = y.pins.iter().map(|p| p.pin.get_ref().clone()).collect();
            (y.reference().to_lowercase(), pins)
        })
        .collect();
    let part_pin = |part: &Option<String>, p: &str| -> Option<String> {
        let part = lib?.get(part.as_deref()?)?;
        pin_named(part, p).map(str::to_owned)
    };
    for net in &mut file.nets {
        for pin in &mut net.pins {
            let Some((r, p)) = split_pin(pin.get_ref()) else {
                continue;
            };
            let Some((written, part)) = canon.get(&r.to_lowercase()) else {
                continue;
            };
            let p_canon = match part {
                Some(_) => part_pin(part, p),
                None => symbol_pins
                    .get(&r.to_lowercase())
                    .and_then(|ps| ps.iter().find(|s| s.eq_ignore_ascii_case(p)).cloned()),
            }
            .unwrap_or_else(|| p.to_owned());
            let new = format!("{written}.{p_canon}");
            if &new != pin.get_ref() {
                *pin = Spanned::new(pin.span(), new);
            }
        }
    }
    if let Some(lib) = lib {
        for c in &mut file.components {
            let Some(part) = lib.get(c.part.get_ref()) else {
                continue;
            };
            for sp in c.schematic.iter_mut().flat_map(|s| s.pins.iter_mut()) {
                if let Some(name) = pin_named(part, sp.pin.get_ref())
                    && name != sp.pin.get_ref()
                {
                    sp.pin = Spanned::new(sp.pin.span(), name.to_owned());
                }
            }
        }
    }
}

/// A firmware file of one component: (ref, path, hash field).
type HashedFile<'a> = (&'a str, &'a Spanned<String>, Option<&'a Spanned<String>>);

/// Firmware files with their hash fields, in file order.
fn hashed_files(file: &CircuitFile) -> Vec<HashedFile<'_>> {
    let mut out = Vec::new();
    for c in &file.components {
        let Some(fw) = &c.firmware else { continue };
        out.push((c.reference(), &fw.app, Some(&fw.app_sha256)));
        if let Some(p) = &fw.partitions {
            out.push((c.reference(), p, fw.partitions_sha256.as_ref()));
        }
    }
    out
}

/// Firmware problems, sorted by position: a file that cannot be read (at its
/// path), or a hash that does not match the file (at the hash). A mismatch is
/// always an error; `ohmforge circuit fmt --update-hashes` rewrites the hash
/// when the firmware change is intended.
pub fn check_firmware(src: &str, file: &CircuitFile, res: &dyn Resolver) -> Vec<CircuitError> {
    let mut out = Vec::new();
    for (r, path, sha) in hashed_files(file) {
        match res.read(path.get_ref()) {
            Err(e) => out.push(CircuitError::at(
                src,
                path.span(),
                format!("{r}: cannot read firmware: {e}"),
            )),
            Ok(bytes) => {
                let Some(sha) = sha else { continue };
                let actual = sha256_hex(&bytes);
                if &actual != sha.get_ref() {
                    out.push(CircuitError::at(
                        src,
                        sha.span(),
                        format!(
                            "{r}: SHA-256 of '{}' is {actual}, not the recorded hash; \
                             if the firmware change is intended, run `ohmforge circuit fmt --update-hashes`",
                            path.get_ref()
                        ),
                    ));
                }
            }
        }
    }
    out.sort_by_key(|e| (e.line, e.column));
    out
}

/// Recompute every firmware and partition-table hash from the files.
/// Returns how many hash fields changed (missing `partitions_sha256` fields are added).
pub fn update_hashes(src: &str, file: &mut CircuitFile, res: &dyn Resolver) -> Result<usize, CircuitError> {
    let mut changed = 0;
    for c in &mut file.components {
        let r = c.reference.get_ref().clone();
        let Some(fw) = &mut c.firmware else { continue };
        let read = |path: &Spanned<String>| {
            res.read(path.get_ref())
                .map(|b| sha256_hex(&b))
                .map_err(|e| CircuitError::at(src, path.span(), format!("{r}: cannot read firmware: {e}")))
        };
        let app = read(&fw.app)?;
        if &app != fw.app_sha256.get_ref() {
            fw.app_sha256 = Spanned::new(fw.app_sha256.span(), app);
            changed += 1;
        }
        if let Some(p) = &fw.partitions {
            let hash = read(p)?;
            match &fw.partitions_sha256 {
                Some(h) if h.get_ref() == &hash => {}
                Some(h) => {
                    fw.partitions_sha256 = Some(Spanned::new(h.span(), hash));
                    changed += 1;
                }
                None => {
                    fw.partitions_sha256 = Some(Spanned::new(p.span(), hash));
                    changed += 1;
                }
            }
        }
    }
    Ok(changed)
}
