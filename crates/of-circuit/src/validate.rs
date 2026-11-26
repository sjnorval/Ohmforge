//! Structural checks that need no parts library (RFC 0003, "Validation").

use std::collections::{BTreeMap, BTreeSet};
use std::ops::Range;

use toml::Spanned;

use crate::{
    CircuitError, CircuitFile, IDEAL_PROFILE, INSTRUMENT_KINDS, InstrumentEntry, MULTIMETER_INPUTS, MULTIMETER_MODES,
    NetKind, ParamValue, SCOPE_PARAMS, SCOPE_TRIGGERS, SYMBOL_KINDS, SchematicPin,
};

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

/// A profile id: `ideal` or `<vendor>.<model>`, each half letters, digits, `_` or `-`.
fn valid_profile(p: &str) -> bool {
    p == IDEAL_PROFILE || p.split_once('.').is_some_and(|(v, m)| valid_ref(v) && valid_ref(m))
}

/// Seconds in a time such as `1ms`, `250us`, `10ns`, `2s` or `1e-3`; `None`
/// unless it is a positive finite time.
pub(crate) fn parse_time(s: &str) -> Option<f64> {
    let t = s.trim();
    let (num, scale) = [("ns", 1e-9), ("us", 1e-6), ("\u{b5}s", 1e-6), ("ms", 1e-3), ("s", 1.0)]
        .iter()
        .find_map(|(suffix, k)| t.strip_suffix(suffix).map(|n| (n, *k)))
        .unwrap_or((t, 1.0));
    let v: f64 = num.trim().parse().ok()?;
    (v.is_finite() && v > 0.0).then_some(v * scale)
}

/// True when an instrument probe entry is an inline signal rather than a probe name.
pub(crate) fn is_inline_signal(s: &str) -> bool {
    s.contains('(')
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
/// hex characters; partitions come with their hash; positive `[sim]` steps;
/// unique, non-empty probe names; instruments (RFC 0004) with a unique ref, a
/// known kind, a mode only on (and always on) a multimeter, a profile id of the
/// right shape, probes that name a `[[probe]]` or are inline signals (a
/// multimeter takes exactly one) and known, well-typed `params`.
///
/// Signal grammar and resolution need the parts library:
/// `library::check_signals` (feature `engine`).
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
    if !sim.ic.is_empty() && !sim.uic {
        s.err(sim_at..sim_at, "[sim] ic needs uic = true");
    }
    for (net, v) in &sim.ic {
        if !v.is_finite() {
            s.err(sim_at..sim_at, format!("[sim] ic.{net} must be finite"));
        }
        if !file.nets.iter().any(|n| n.name.get_ref() == net) {
            s.err(sim_at..sim_at, format!("[sim] ic names unknown net '{net}'"));
        }
    }

    // Refs.
    let mut seen: BTreeMap<String, &str> = BTreeMap::new();
    let all_refs = file
        .components
        .iter()
        .map(|c| &c.reference)
        .chain(file.symbols.iter().map(|y| &y.reference))
        .chain(file.instruments.iter().map(|i| &i.reference));
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

    // Probes and instruments.
    let mut probe_names = BTreeSet::new();
    for p in &file.probes {
        let name = p.name();
        if name.trim().is_empty() {
            s.err(p.name.span(), "probe name is empty");
        } else if !probe_names.insert(name) {
            s.err(p.name.span(), format!("duplicate probe name '{name}'"));
        }
    }
    for i in &file.instruments {
        check_instrument(&mut s, i, &probe_names);
    }

    let mut out = s.out;
    out.sort_by_key(|e| (e.line, e.column));
    out
}

fn check_instrument(s: &mut Sink<'_>, i: &InstrumentEntry, probe_names: &BTreeSet<&str>) {
    let r = i.reference();
    let kind = i.kind();
    if !INSTRUMENT_KINDS.contains(&kind) {
        s.err(
            i.kind.span(),
            format!(
                "{r}: unknown instrument kind '{kind}' (expected {})",
                INSTRUMENT_KINDS.join(", ")
            ),
        );
    }
    match (&i.mode, kind == "multimeter") {
        (None, true) => s.err(
            i.kind.span(),
            format!("{r}: a multimeter needs a mode ({})", MULTIMETER_MODES.join(", ")),
        ),
        (Some(m), true) if !MULTIMETER_MODES.contains(&m.get_ref().as_str()) => s.err(
            m.span(),
            format!(
                "{r}: unknown multimeter mode '{}' (expected {})",
                m.get_ref(),
                MULTIMETER_MODES.join(", ")
            ),
        ),
        (Some(m), false) => s.err(m.span(), format!("{r}: only a multimeter has a mode")),
        _ => {}
    }
    if let Some(p) = &i.profile
        && !valid_profile(p.get_ref())
    {
        s.err(
            p.span(),
            format!(
                "{r}: profile '{}' must be \"{IDEAL_PROFILE}\" or <vendor>.<model>",
                p.get_ref()
            ),
        );
    }
    if i.probes.is_empty() {
        s.err(i.reference.span(), format!("{r}: probes is empty"));
    }
    if kind == "multimeter" && i.probes.len() > 1 {
        s.err(
            i.probes[1].span(),
            format!("{r}: a multimeter reads one signal (use v(a,b) for a voltage between two nets)"),
        );
    }
    if kind == "uart" && i.probes.len() > 2 {
        s.err(
            i.probes[2].span(),
            format!("{r}: a UART console reads TX and optionally RX, e.g. [\"d(U1.TX)\", \"d(U1.RX)\"]"),
        );
    }
    for p in &i.probes {
        let text = p.get_ref();
        if !is_inline_signal(text) && !probe_names.contains(text.as_str()) {
            s.err(
                p.span(),
                format!("{r}: no [[probe]] named '{text}' (inline signals look like v(net) or i(R1))"),
            );
        }
    }
    for (k, v) in &i.params {
        let problem = match (kind, k.as_str(), v) {
            ("scope", "time_div", ParamValue::Text(t)) if parse_time(t).is_some() => None,
            ("scope", "time_div", _) => Some("time_div must be a time such as \"1ms\"".to_owned()),
            ("scope", "trigger", ParamValue::Text(t)) if SCOPE_TRIGGERS.contains(&t.as_str()) => None,
            ("scope", "trigger", _) => Some(format!("trigger must be one of {}", SCOPE_TRIGGERS.join(", "))),
            ("scope", "level", ParamValue::Number(x)) if x.is_finite() => None,
            ("scope", "level", _) => Some("level must be a number (volts or amps of the first channel)".into()),
            ("multimeter", "input", ParamValue::Text(t)) if MULTIMETER_INPUTS.contains(&t.as_str()) => None,
            ("multimeter", "input", _) => Some(format!("input must be one of {}", MULTIMETER_INPUTS.join(", "))),
            ("uart", "baud", ParamValue::Number(x)) if x.is_finite() && *x > 0.0 => None,
            ("uart", "baud", _) => Some("baud must be a positive number, e.g. 115200".into()),
            ("logic", "sample_rate", ParamValue::Number(x)) if x.is_finite() && *x > 0.0 => None,
            ("logic", "sample_rate", _) => Some("sample_rate must be a positive number in Hz, e.g. 25e6".into()),
            ("scope", other, _) => Some(format!(
                "unknown scope param '{other}' (expected {})",
                SCOPE_PARAMS.join(", ")
            )),
            (_, other, _) => Some(format!("unknown {kind} param '{other}'")),
        };
        if let Some(problem) = problem {
            s.err(i.reference.span(), format!("{r}: {problem}"));
        }
    }
}

/// The first structural problem, if any.
pub(crate) fn validate(src: &str, file: &CircuitFile) -> Result<(), CircuitError> {
    match diagnostics(src, file).into_iter().next() {
        Some(e) => Err(e),
        None => Ok(()),
    }
}
