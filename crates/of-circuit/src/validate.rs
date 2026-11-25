//! Structural checks that need no parts library (RFC 0003, "Validation").

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use toml::Spanned;

use crate::{CircuitError, CircuitFile, NetKind, SYMBOL_KINDS, SchematicPin};

/// What a ref names.
#[derive(Clone, Copy)]
#[cfg_attr(not(feature = "engine"), allow(dead_code))] // the component index is read by `library`
pub(crate) enum RefTarget {
    /// Index into `components`.
    Component(usize),
    /// Index into `symbols`.
    Symbol(usize),
}

/// Refs by lower-case name: (target, ref as written).
pub(crate) fn ref_table(file: &CircuitFile) -> BTreeMap<String, (RefTarget, &str)> {
    let mut refs = BTreeMap::new();
    for (i, c) in file.components.iter().enumerate() {
        refs.entry(c.reference().to_lowercase())
            .or_insert((RefTarget::Component(i), c.reference()));
    }
    for (i, s) in file.symbols.iter().enumerate() {
        refs.entry(s.reference().to_lowercase())
            .or_insert((RefTarget::Symbol(i), s.reference()));
    }
    refs
}

/// Split `REF.PIN` at the first dot; `None` unless both halves are non-empty.
pub(crate) fn split_pin(s: &str) -> Option<(&str, &str)> {
    s.split_once('.').filter(|(r, p)| !r.is_empty() && !p.is_empty())
}

/// Byte offset of a `[name]` table header line, or 0.
pub(crate) fn header_offset(src: &str, header: &str) -> usize {
    let mut offset = 0;
    for line in src.split_inclusive('\n') {
        let trimmed = line.trim_start();
        if trimmed.starts_with(header) {
            return offset + (line.len() - trimmed.len());
        }
        offset += line.len();
    }
    0
}

/// Span of a value, or of a table header when the value was defaulted.
fn span_or_header(src: &str, v: &Spanned<String>, header: &str) -> Range<usize> {
    let s = v.span();
    if s == (0..0) {
        let at = header_offset(src, header);
        at..at
    } else {
        s
    }
}

fn valid_ref(r: &str) -> bool {
    !r.is_empty() && r.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
}

fn valid_sha(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

/// Problem with a firmware path, if any.
fn path_problem(p: &str) -> Option<&'static str> {
    if p.is_empty() {
        Some("firmware path is empty")
    } else if p.contains('\\') {
        Some("firmware paths use '/' as the separator")
    } else if p.starts_with('/') || p.contains(':') {
        Some("firmware paths must be relative to the circuit file")
    } else {
        None
    }
}

struct Sink<'a> {
    src: &'a str,
    out: Vec<CircuitError>,
}

impl Sink<'_> {
    fn err(&mut self, span: Range<usize>, msg: impl Into<String>) {
        self.out.push(CircuitError::at(self.src, span, msg));
    }
}

/// Every structural problem in `file`, sorted by position (file order).
///
/// Checks: unique refs across components and symbols (case-insensitive, letters,
/// digits, `_` and `-` only); net pins are `REF.PIN` with a known ref (and a
/// symbol's own pin for symbols); a `REF.PIN` sits on at most one net, once;
/// unique net names (case-insensitive); exactly one net named `[circuit] ground`,
/// and it alone has `kind = "ground"`; every routed schematic pin is on a net and
/// drawn once; slide parameters are numeric when set; known symbol kinds;
/// firmware paths are relative with `/` separators and hashes are 64 lower-case
/// hex characters; partitions come with their hash; positive `[sim]` steps.
pub fn diagnostics(src: &str, file: &CircuitFile) -> Vec<CircuitError> {
    let mut s = Sink { src, out: Vec::new() };

    // Sim settings.
    let sim = &file.sim;
    let sim_at = header_offset(src, "[sim]");
    if sim.quantum_us == 0 {
        s.err(sim_at..sim_at, "[sim] quantum_us must be at least 1");
    }
    if !(sim.max_step.is_finite() && sim.max_step > 0.0) {
        s.err(sim_at..sim_at, "[sim] max_step must be a positive number of seconds");
    }
    if sim.temperature_c.is_some_and(|t| !t.is_finite()) {
        s.err(sim_at..sim_at, "[sim] temperature_c must be finite");
    }

    // Refs.
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let all_refs = file
        .components
        .iter()
        .map(|c| &c.reference)
        .chain(file.symbols.iter().map(|y| &y.reference));
    for r in all_refs {
        let name = r.get_ref();
        if !valid_ref(name) {
            s.err(r.span(), format!("ref '{name}' must be letters, digits, '_' or '-'"));
        }
        if let Some(first) = seen.get(&name.to_lowercase()) {
            s.err(r.span(), format!("duplicate ref '{name}' (already used by '{first}')"));
        } else {
            seen.insert(name.to_lowercase(), name);
        }
    }
    let refs = ref_table(file);

    // Symbols.
    for y in &file.symbols {
        if !SYMBOL_KINDS.contains(&y.kind.get_ref().as_str()) {
            s.err(
                y.kind.span(),
                format!(
                    "unknown symbol kind '{}' (expected {})",
                    y.kind.get_ref(),
                    SYMBOL_KINDS.join(", ")
                ),
            );
        }
    }

    // Nets and their pins.
    let mut net_names: BTreeMap<String, &str> = BTreeMap::new();
    let mut on_net: BTreeMap<(String, String), &str> = BTreeMap::new();
    for net in &file.nets {
        let name = net.name();
        if name.is_empty() {
            s.err(net.name.span(), "net name is empty");
        }
        if net_names.insert(name.to_lowercase(), name).is_some() {
            s.err(net.name.span(), format!("duplicate net name '{name}'"));
        }
        for pin in &net.pins {
            let text = pin.get_ref();
            let Some((r, p)) = split_pin(text) else {
                s.err(pin.span(), format!("expected REF.PIN, got '{text}'"));
                continue;
            };
            let Some(&(target, _)) = refs.get(&r.to_lowercase()) else {
                s.err(pin.span(), format!("'{text}': unknown ref '{r}'"));
                continue;
            };
            if let RefTarget::Symbol(i) = target {
                let y = &file.symbols[i];
                if !y.pins.iter().any(|sp| sp.pin.get_ref().eq_ignore_ascii_case(p)) {
                    s.err(
                        pin.span(),
                        format!("'{text}': symbol '{}' has no pin '{p}'", y.reference()),
                    );
                    continue;
                }
            }
            let key = (r.to_lowercase(), p.to_lowercase());
            match on_net.get(&key) {
                Some(&other) if other == name => {
                    s.err(pin.span(), format!("'{text}' is listed twice on net '{name}'"));
                }
                Some(&other) => {
                    s.err(pin.span(), format!("'{text}' is already on net '{other}'"));
                }
                None => {
                    on_net.insert(key, name);
                }
            }
        }
    }

    // Ground.
    let ground = file.circuit.ground.get_ref();
    let ground_span = span_or_header(src, &file.circuit.ground, "[circuit]");
    match file.nets.iter().find(|n| n.name().eq_ignore_ascii_case(ground)) {
        None => s.err(
            ground_span,
            format!("ground net '{ground}' has no [[net]] (declare it with kind = \"ground\")"),
        ),
        Some(n) if n.kind != NetKind::Ground => s.err(
            n.name.span(),
            format!("net '{}' is the ground net and needs kind = \"ground\"", n.name()),
        ),
        Some(_) => {}
    }
    for n in &file.nets {
        if n.kind == NetKind::Ground && !n.name().eq_ignore_ascii_case(ground) {
            s.err(
                n.name.span(),
                format!(
                    "only the ground net '{ground}' may have kind = \"ground\" (set [circuit] ground to change it)"
                ),
            );
        }
    }

    // Schematic pins.
    let check_pins = |s: &mut Sink<'_>, r: &str, pins: &[SchematicPin]| {
        let mut drawn = BTreeSet::new();
        for sp in pins {
            let p = sp.pin.get_ref();
            if !drawn.insert(p.to_lowercase()) {
                s.err(sp.pin.span(), format!("{r}: pin '{p}' is drawn twice"));
                continue;
            }
            if !sp.stub && !on_net.contains_key(&(r.to_lowercase(), p.to_lowercase())) {
                s.err(
                    sp.pin.span(),
                    format!("{r}.{p} is drawn but not on any net (connect it or set stub = true)"),
                );
            }
        }
    };
    for c in &file.components {
        let Some(sch) = &c.schematic else { continue };
        check_pins(&mut s, c.reference(), &sch.pins);
        for sp in &sch.pins {
            let Some(slide) = &sp.slide else { continue };
            let param = slide.param.get_ref();
            if c.params.get(param).is_some_and(|v| v.as_number().is_none()) {
                s.err(
                    slide.param.span(),
                    format!("{}: slide parameter '{param}' is not a number", c.reference()),
                );
            }
        }
    }
    for y in &file.symbols {
        check_pins(&mut s, y.reference(), &y.pins);
        for sp in &y.pins {
            if let Some(slide) = &sp.slide {
                s.err(
                    slide.param.span(),
                    format!("{}: symbol pins cannot slide", y.reference()),
                );
            }
        }
    }

    // Firmware references.
    for c in &file.components {
        let Some(fw) = &c.firmware else { continue };
        let paths = std::iter::once((&fw.app, Some(&fw.app_sha256), "app_sha256")).chain(
            fw.partitions
                .iter()
                .map(|p| (p, fw.partitions_sha256.as_ref(), "partitions_sha256")),
        );
        for (path, sha, sha_key) in paths {
            if let Some(problem) = path_problem(path.get_ref()) {
                s.err(path.span(), format!("{}: {problem}", c.reference()));
            }
            match sha {
                None => s.err(
                    path.span(),
                    format!("{}: partitions needs a {sha_key} next to it", c.reference()),
                ),
                Some(h) if !valid_sha(h.get_ref()) => s.err(
                    h.span(),
                    format!(
                        "{}: {sha_key} must be 64 lower-case hex characters (SHA-256)",
                        c.reference()
                    ),
                ),
                Some(_) => {}
            }
        }
        if let (None, Some(h)) = (&fw.partitions, &fw.partitions_sha256) {
            s.err(
                h.span(),
                format!("{}: partitions_sha256 without partitions", c.reference()),
            );
        }
    }

    let mut out = s.out;
    out.sort_by_key(|e| (e.line, e.column));
    out
}

/// The first structural problem, if any.
pub(crate) fn validate(src: &str, file: &CircuitFile) -> Result<(), CircuitError> {
    match diagnostics(src, file).into_iter().next() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
