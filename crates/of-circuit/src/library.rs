//! Checks against the parts library and firmware files (feature `engine`).
//!
//! File access goes through [`Resolver`], so these checks also run in wasm32
//! with an embedded resolver; [`crate::fs::FsResolver`] reads from disk.

use std::collections::BTreeMap;

use of_instruments::Signal;
use of_parts::PartFile;
use toml::Spanned;

use crate::validate::{RefTarget, is_inline_signal, ref_table, split_pin};
use crate::{CircuitError, CircuitFile, IDEAL_PROFILE};

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

/// Reads files a circuit refers to (firmware, partition tables) and, for
/// `boot = "rom"`, the chip's fetched mask ROM and bootloader.
pub trait Resolver {
    /// Bytes of `path` as written in the file (relative to the circuit file,
    /// `/` separated). The error text names the file.
    fn read(&self, path: &str) -> Result<Vec<u8>, String>;

    /// Mask ROM ELF and second-stage bootloader for library part `part`
    /// (`boot = "rom"`). They are fetched, never in the repository; the
    /// default has none.
    fn rom(&self, part: &str) -> Result<RomFiles, String> {
        Err(format!(
            "no mask ROM is available for {part}: this resolver does not provide one"
        ))
    }

    /// An already booted chip for a `boot = "rom"` placement, for example a
    /// desktop pre-boot cache keyed by the app hash and the ready text.
    /// `Ok(None)` (the default) boots from the ROM.
    fn booted(&self, _part: &str, _app_sha256: &str, _ready_text: &str) -> Result<Option<of_engine::Firmware>, String> {
        Ok(None)
    }
}

/// A chip's mask ROM ELF and second-stage bootloader ELF.
#[derive(Clone, PartialEq, Eq)]
pub struct RomFiles {
    /// Mask ROM ELF.
    pub rom: Vec<u8>,
    /// Second-stage bootloader ELF.
    pub bootloader: Vec<u8>,
}

impl std::fmt::Debug for RomFiles {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "RomFiles {{ rom: {} bytes, bootloader: {} bytes }}",
            self.rom.len(),
            self.bootloader.len()
        )
    }
}

/// Lower-case hex SHA-256.
pub fn sha256_hex(bytes: &[u8]) -> String {
    of_firmware::sha256(bytes).iter().map(|b| format!("{b:02x}")).collect()
}

/// The part pin with this name (case-insensitive). Pin numbers do not match:
/// circuit files use names.
pub(crate) fn pin_named<'a>(part: &'a PartFile, name: &str) -> Option<&'a str> {
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

/// Problems with the file's signals (RFC 0004), sorted by position: every
/// `[[probe]] signal` and inline instrument signal must parse and resolve.
///
/// - `v(net)` and `v(a,b)` name declared nets (or `0` for ground);
/// - `i(REF)` needs a two-pin part, `i(REF.PIN)`, `d(REF.PIN)` a pin of the part;
/// - `r(REF.reading)` needs a component (reading names are the component's own and
///   are only known once it runs);
/// - names are case-sensitive, as the running session reads them;
/// - a multimeter in `dc_v` reads a `v(..)`, in `dc_a` an `i(..)`; a scope reads
///   analog signals, a logic analyser `d(..)`;
/// - a profile is `"ideal"` or a built-in real-world profile
///   (`of_instruments::profile::lookup`) of the instrument's kind.
///
/// Parts missing from `lib` are reported by [`check_parts`], not here.
pub fn check_signals(src: &str, file: &CircuitFile, lib: &PartsIndex) -> Vec<CircuitError> {
    let mut out = Vec::new();
    let mut err = |span: std::ops::Range<usize>, msg: String| out.push(CircuitError::at(src, span, msg));
    let mut signals = BTreeMap::new();
    for p in &file.probes {
        match resolve(file, lib, p.signal()) {
            Ok(sig) => {
                signals.insert(p.name(), sig);
            }
            Err(e) => err(p.signal.span(), format!("probe '{}': {e}", p.name())),
        }
    }
    for i in &file.instruments {
        let r = i.reference();
        if i.profile() != IDEAL_PROFILE
            && let Some(p) = &i.profile
        {
            match of_instruments::profile::lookup(p.get_ref()) {
                None => {
                    let known: Vec<String> = of_instruments::profile::builtins()
                        .iter()
                        .filter(|k| k.kind().is_some())
                        .map(|k| format!("\"{}\"", k.id()))
                        .collect();
                    err(
                        p.span(),
                        format!(
                            "{r}: unknown instrument profile '{}' (known: \"{IDEAL_PROFILE}\", {})",
                            p.get_ref(),
                            known.join(", ")
                        ),
                    );
                }
                Some(k) if k.kind() != Some(i.kind()) => err(
                    p.span(),
                    format!(
                        "{r}: '{}' is a {} profile, not a {} one",
                        p.get_ref(),
                        k.kind().unwrap_or("bench-supply"),
                        i.kind()
                    ),
                ),
                Some(_) => {}
            }
        }
        for p in &i.probes {
            let text = p.get_ref();
            let sig = if is_inline_signal(text) {
                match resolve(file, lib, text) {
                    Ok(sig) => sig,
                    Err(e) => {
                        err(p.span(), format!("{r}: {e}"));
                        continue;
                    }
                }
            } else {
                match signals.get(text.as_str()) {
                    Some(sig) => sig.clone(),
                    None => continue, // unknown names and bad probes are reported elsewhere
                }
            };
            if let Some(problem) = kind_mismatch(i.kind(), i.mode(), &sig) {
                err(p.span(), format!("{r}: {problem}"));
            }
        }
    }
    out.sort_by_key(|e| (e.line, e.column));
    out
}

/// Why `sig` does not suit an instrument of `kind` (and `mode`), if it does not.
fn kind_mismatch(kind: &str, mode: Option<&str>, sig: &Signal) -> Option<String> {
    let volts = matches!(sig, Signal::NetVolts(_) | Signal::DiffVolts(..));
    let amps = matches!(sig, Signal::PinCurrent { .. });
    let digital = matches!(sig, Signal::Digital { .. });
    match (kind, mode) {
        ("multimeter", Some("dc_v")) if !volts => Some(format!("a dc_v multimeter reads v(..), not {sig}")),
        ("multimeter", Some("dc_a")) if !amps => Some(format!("a dc_a multimeter reads i(..), not {sig}")),
        ("scope", _) if digital => Some(format!("a scope channel is analog; {sig} belongs on a logic analyser")),
        ("logic", _) if !digital => Some(format!("a logic analyser reads d(..), not {sig}")),
        ("uart", _) if !digital => Some(format!("a UART console reads d(..) (TX, then RX), not {sig}")),
        _ => None,
    }
}

/// Parse `text` and check that every name in it exists in `file` (and `lib`).
fn resolve(file: &CircuitFile, lib: &PartsIndex, text: &str) -> Result<Signal, String> {
    let sig: Signal = text
        .parse()
        .map_err(|e: of_instruments::ParseSignalError| e.to_string())?;
    match &sig {
        Signal::NetVolts(n) => net(file, n)?,
        Signal::DiffVolts(a, b) => {
            net(file, a)?;
            net(file, b)?;
        }
        Signal::PinCurrent { refdes, pin: None } => {
            if let Some(part) = component_part(file, lib, refdes)?
                && part.pins.len() != 2
            {
                return Err(format!(
                    "i({refdes}) needs a two-pin part; '{}' has {} pins (name one: i({refdes}.PIN))",
                    part.part.id,
                    part.pins.len()
                ));
            }
        }
        Signal::PinCurrent { refdes, pin: Some(pin) } | Signal::Digital { refdes, pin } => {
            if let Some(part) = component_part(file, lib, refdes)? {
                match pin_named(part, pin) {
                    Some(p) if p == pin => {}
                    Some(p) => {
                        return Err(format!(
                            "pin '{pin}' of {refdes} is written '{p}' (names are case-sensitive)"
                        ));
                    }
                    None => {
                        return Err(format!(
                            "part '{}' of {refdes} has no pin '{pin}' (pins: {})",
                            part.part.id,
                            pin_list(part)
                        ));
                    }
                }
            }
        }
        Signal::Reading { refdes, .. } => {
            component_part(file, lib, refdes)?;
        }
    }
    Ok(sig)
}

/// `Ok` when `name` is a declared net (or `0`, the solver's ground).
fn net(file: &CircuitFile, name: &str) -> Result<(), String> {
    if name == "0" || file.nets.iter().any(|n| n.name() == name) {
        return Ok(());
    }
    match file.nets.iter().find(|n| n.name().eq_ignore_ascii_case(name)) {
        Some(n) => Err(format!(
            "net '{name}' is written '{}' (names are case-sensitive)",
            n.name()
        )),
        None => Err(format!("no net '{name}'")),
    }
}

/// The library part of component `refdes` (`None` when the part is not in `lib`).
fn component_part<'a>(file: &CircuitFile, lib: &'a PartsIndex, refdes: &str) -> Result<Option<&'a PartFile>, String> {
    if let Some(c) = file.components.iter().find(|c| c.reference() == refdes) {
        return Ok(lib.get(c.part_id()));
    }
    if let Some(c) = file
        .components
        .iter()
        .find(|c| c.reference().eq_ignore_ascii_case(refdes))
    {
        return Err(format!(
            "component '{refdes}' is written '{}' (names are case-sensitive)",
            c.reference()
        ));
    }
    Err(format!("no component '{refdes}'"))
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

/// A part on a supply rail that states its supply range (`[electrical]`
/// `vdd` or `vcc`, `min` and `max`).
#[derive(Debug, Clone, PartialEq)]
pub struct RailPart {
    /// Reference designator, for example `U1`.
    pub reference: String,
    /// Part id, for example `ti.ne555`.
    pub part: String,
    /// Lowest supply (V).
    pub min: f64,
    /// Highest supply (V).
    pub max: f64,
}

/// The parts on net `rail` (other than `skip`, the supply itself) that
/// state a supply range, in pin order.
pub fn rail_parts(file: &CircuitFile, rail: &str, skip: &str, lib: &PartsIndex) -> Vec<RailPart> {
    let mut out = Vec::new();
    let Some(net) = file.nets.iter().find(|n| n.name() == rail) else {
        return out;
    };
    for p in &net.pins {
        let Some((r, _)) = split_pin(p.get_ref()) else {
            continue;
        };
        if r.eq_ignore_ascii_case(skip) || out.iter().any(|x: &RailPart| x.reference.eq_ignore_ascii_case(r)) {
            continue;
        }
        let Some(c) = file.components.iter().find(|c| c.reference().eq_ignore_ascii_case(r)) else {
            continue;
        };
        let Some(part) = lib.get(c.part_id()) else {
            continue;
        };
        for key in ["vdd", "vcc"] {
            let Some(t) = part.electrical.get(key).and_then(|v| v.as_table()) else {
                continue;
            };
            let num = |k: &str| {
                t.get(k)
                    .and_then(|v| v.as_float().or_else(|| v.as_integer().map(|i| i as f64)))
            };
            if let (Some(min), Some(max)) = (num("min"), num("max")) {
                out.push(RailPart {
                    reference: c.reference().to_owned(),
                    part: c.part_id().to_owned(),
                    min,
                    max,
                });
                break;
            }
        }
    }
    out
}

/// Warnings for bench supplies whose + rail carries parts with no common
/// supply range, naming them, for example `PSU: parts on this rail have no
/// common supply range (U1 espressif.esp32-c3 3.0–3.6 V, U2 ti.ne555
/// 4.5–16.0 V)`.
pub fn rail_warnings(file: &CircuitFile, lib: &PartsIndex) -> Vec<String> {
    let mut out = Vec::new();
    for c in &file.components {
        if lib.get(c.part_id()).is_none_or(|p| p.part.model != "rust:bench-psu") {
            continue;
        }
        let plus = format!("{}.+", c.reference());
        let Some(rail) = file
            .nets
            .iter()
            .find(|n| n.pins.iter().any(|p| p.get_ref().eq_ignore_ascii_case(&plus)))
        else {
            continue;
        };
        let on_rail = rail_parts(file, rail.name(), c.reference(), lib);
        let lo = on_rail.iter().map(|p| p.min).fold(f64::MIN, f64::max);
        let hi = on_rail.iter().map(|p| p.max).fold(f64::MAX, f64::min);
        if on_rail.len() > 1 && lo > hi {
            let list: Vec<String> = on_rail
                .iter()
                .map(|p| format!("{} {} {:?}–{:?} V", p.reference, p.part, p.min, p.max))
                .collect();
            out.push(format!(
                "{}: parts on this rail have no common supply range ({})",
                c.reference(),
                list.join(", ")
            ));
        }
    }
    out
}
