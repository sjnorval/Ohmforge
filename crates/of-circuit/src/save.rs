//! Canonical writer. The layout is fixed so `save(parse(text)) == text` for
//! files already in canonical form (RFC 0003, "Canonical form").

use std::fmt::Write as _;

use crate::{
    CircuitFile, ComponentEntry, FirmwareRef, InstrumentEntry, Keepout, NetEntry, ParamValue, SchematicPin,
    SimSettings, SymbolEntry,
};

/// Canonical text for `file`. Comments are not kept; use `note` and
/// `description` fields for anything worth keeping.
pub fn save(file: &CircuitFile) -> String {
    let mut w = String::new();
    let _ = writeln!(w, "format = {}", file.format.get_ref());

    w.push_str("\n[circuit]\n");
    kv(&mut w, "name", &string(&file.circuit.name));
    kv(&mut w, "ground", &string(file.circuit.ground.get_ref()));
    if let Some(d) = &file.circuit.description {
        kv(&mut w, "description", &string(d));
    }

    write_sim(&mut w, &file.sim);
    for c in &file.components {
        write_component(&mut w, c);
    }
    for y in &file.symbols {
        write_symbol(&mut w, y);
    }
    for n in &file.nets {
        write_net(&mut w, n);
    }
    if !file.schematic.keepouts.is_empty() {
        w.push_str("\n[schematic]\nkeepouts = [\n");
        for k in &file.schematic.keepouts {
            let _ = writeln!(w, "  {},", keepout(k));
        }
        w.push_str("]\n");
    }
    for p in &file.probes {
        w.push_str("\n[[probe]]\n");
        kv(&mut w, "name", &string(p.name()));
        kv(&mut w, "signal", &string(p.signal()));
    }
    for i in &file.instruments {
        write_instrument(&mut w, i);
    }
    w
}

/// True when both files have the same content (spans and layout are ignored):
/// `save(a) == save(b)`.
pub fn same_content(a: &CircuitFile, b: &CircuitFile) -> bool {
    save(a) == save(b)
}

fn kv(w: &mut String, key: &str, value: &str) {
    let _ = writeln!(w, "{key} = {value}");
}

fn write_sim(w: &mut String, sim: &SimSettings) {
    let d = SimSettings::default();
    let mut lines = Vec::new();
    if sim.quantum_us != d.quantum_us {
        lines.push(format!("quantum_us = {}", sim.quantum_us));
    }
    if sim.max_step.to_bits() != d.max_step.to_bits() {
        lines.push(format!("max_step = {}", number(sim.max_step)));
    }
    if sim.corner != d.corner {
        lines.push(format!("corner = {}", string(sim.corner.as_str())));
    }
    if let Some(t) = sim.temperature_c {
        lines.push(format!("temperature_c = {}", number(t)));
    }
    if sim.seed != d.seed {
        lines.push(format!("seed = {}", sim.seed));
    }
    if !lines.is_empty() {
        w.push_str("\n[sim]\n");
        for l in lines {
            w.push_str(&l);
            w.push('\n');
        }
    }
}

fn write_component(w: &mut String, c: &ComponentEntry) {
    w.push_str("\n[[component]]\n");
    kv(w, "ref", &string(c.reference()));
    kv(w, "part", &string(c.part_id()));
    if let Some(at) = c.at {
        kv(w, "at", &coords(&at));
    }
    if !c.params.is_empty() {
        let items: Vec<String> = c
            .params
            .iter()
            .map(|(k, v)| format!("{} = {}", key(k), param(v)))
            .collect();
        kv(w, "params", &format!("{{ {} }}", items.join(", ")));
    }
    if let Some(seed) = c.seed {
        kv(w, "seed", &seed.to_string());
    }
    if let Some(fw) = &c.firmware {
        write_firmware(w, fw);
    }
    if let Some(sch) = &c.schematic {
        w.push_str("\n[component.schematic]\n");
        if let Some(id) = &sch.id {
            kv(w, "id", &string(id));
        }
        if let Some(label) = &sch.label {
            kv(w, "label", &string(label));
        }
        kv(w, "body", &coords(&sch.body));
        write_pins(w, &sch.pins);
    }
}

fn write_instrument(w: &mut String, i: &InstrumentEntry) {
    w.push_str("\n[[instrument]]\n");
    kv(w, "ref", &string(i.reference()));
    kv(w, "kind", &string(i.kind()));
    if let Some(p) = &i.profile {
        kv(w, "profile", &string(p.get_ref()));
    }
    if let Some(m) = i.mode() {
        kv(w, "mode", &string(m));
    }
    let probes: Vec<String> = i.probes.iter().map(|p| string(p.get_ref())).collect();
    kv(w, "probes", &format!("[{}]", probes.join(", ")));
    if let Some(at) = i.at {
        kv(w, "at", &coords(&at));
    }
    if !i.params.is_empty() {
        let items: Vec<String> = i
            .params
            .iter()
            .map(|(k, v)| format!("{} = {}", key(k), param(v)))
            .collect();
        kv(w, "params", &format!("{{ {} }}", items.join(", ")));
    }
}

fn write_firmware(w: &mut String, fw: &FirmwareRef) {
    w.push_str("\n[component.firmware]\n");
    kv(w, "app", &string(fw.app.get_ref()));
    kv(w, "app_sha256", &string(fw.app_sha256.get_ref()));
    kv(w, "boot", &string(fw.boot.as_str()));
    if let Some(p) = &fw.partitions {
        kv(w, "partitions", &string(p.get_ref()));
    }
    if let Some(h) = &fw.partitions_sha256 {
        kv(w, "partitions_sha256", &string(h.get_ref()));
    }
    if let Some(t) = &fw.ready_text {
        kv(w, "ready_text", &string(t));
    }
}

fn write_symbol(w: &mut String, y: &SymbolEntry) {
    w.push_str("\n[[symbol]]\n");
    kv(w, "ref", &string(y.reference()));
    kv(w, "kind", &string(y.kind.get_ref()));
    if let Some(id) = &y.id {
        kv(w, "id", &string(id));
    }
    kv(w, "at", &coords(&y.at));
    kv(w, "body", &coords(&y.body));
    write_pins(w, &y.pins);
}

fn write_pins(w: &mut String, pins: &[SchematicPin]) {
    if pins.is_empty() {
        w.push_str("pins = []\n");
        return;
    }
    w.push_str("pins = [\n");
    for p in pins {
        let mut items = vec![format!("pin = {}", string(p.pin.get_ref()))];
        if let Some(id) = &p.id {
            items.push(format!("id = {}", string(id)));
        }
        if let Some(name) = &p.name {
            items.push(format!("name = {}", string(name)));
        }
        items.push(format!("at = {}", coords(&p.at)));
        items.push(format!("dir = {}", string(p.dir.as_str())));
        if p.stub {
            items.push("stub = true".into());
        }
        if let Some(s) = &p.slide {
            items.push(format!(
                "slide = {{ to = {}, param = {} }}",
                coords(&s.to),
                string(s.param.get_ref())
            ));
        }
        let _ = writeln!(w, "  {{ {} }},", items.join(", "));
    }
    w.push_str("]\n");
}

fn write_net(w: &mut String, n: &NetEntry) {
    w.push_str("\n[[net]]\n");
    kv(w, "name", &string(n.name()));
    if let Some(label) = &n.label {
        kv(w, "label", &string(label));
    }
    kv(w, "kind", &string(n.kind.as_str()));
    let pins: Vec<String> = n.pins().map(string).collect();
    kv(w, "pins", &format!("[{}]", pins.join(", ")));
    if !n.waypoints.is_empty() {
        let pts: Vec<String> = n.waypoints.iter().map(|p| coords(p)).collect();
        kv(w, "waypoints", &format!("[{}]", pts.join(", ")));
    }
}

fn keepout(k: &Keepout) -> String {
    match &k.note {
        Some(note) => format!("{{ rect = {}, note = {} }}", coords(&k.rect), string(note)),
        None => format!("{{ rect = {} }}", coords(&k.rect)),
    }
}

fn param(v: &ParamValue) -> String {
    match v {
        ParamValue::Number(x) => number(*x),
        ParamValue::Bool(b) => b.to_string(),
        ParamValue::Text(t) => string(t),
        ParamValue::List(items) => {
            let items: Vec<String> = items.iter().map(|s| string(s)).collect();
            format!("[{}]", items.join(", "))
        }
    }
}

/// Bare key when TOML allows it, else a quoted key.
fn key(k: &str) -> String {
    if !k.is_empty() && k.chars().all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-') {
        k.to_owned()
    } else {
        string(k)
    }
}

/// Non-finite values in TOML spelling, or `None`.
fn special(v: f64) -> Option<&'static str> {
    if v.is_nan() {
        Some("nan")
    } else if v == f64::INFINITY {
        Some("inf")
    } else if v == f64::NEG_INFINITY {
        Some("-inf")
    } else {
        None
    }
}

/// A physical value: integral values without `.0`, very small or very large
/// values as `{:e}` (`5e-6`), everything else in the shortest form that
/// reads back to the same `f64`.
pub(crate) fn number(v: f64) -> String {
    if let Some(s) = special(v) {
        return s.into();
    }
    let a = v.abs();
    if v == v.trunc() && a < 1e15 {
        format!("{}", v as i64)
    } else if a < 1e-3 || a >= 1e15 {
        format!("{v:e}")
    } else {
        format!("{v}")
    }
}

/// A coordinate, like `fmt_coord` in the schematic goldens.
fn coord(v: f32) -> String {
    if let Some(s) = special(f64::from(v)) {
        return s.into();
    }
    if v == v.trunc() && v.abs() < 1e9 {
        format!("{}", v as i64)
    } else {
        format!("{v}")
    }
}

fn coords(v: &[f32]) -> String {
    let parts: Vec<String> = v.iter().map(|&c| coord(c)).collect();
    format!("[{}]", parts.join(", "))
}

/// TOML basic string. Non-ASCII stays UTF-8; control characters are escaped.
pub(crate) fn string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\t' => out.push_str("\\t"),
            '\r' => out.push_str("\\r"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c.is_control() => {
                let _ = write!(out, "\\u{:04X}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
    out
}
