//! Ref and pin naming between the circuit files (`circuits/demos`) and the
//! hand-written boards (E4b-2). The files carry the board's schematic ids,
//! labels, pin ids and names next to the part's electrical pin names (`3V3`
//! is drawn as `VDD` with id `vdd`, PSU `+`/`-` as `plus`/`minus`, the pot
//! wiper `W` as `wiper`, the switch's `1`/`2` as `a`/`b`). This test maps a
//! file onto the schematic model the way E4c's `from_circuit` will and
//! requires the unrouted legacy board exactly: same components in the same
//! order, bodies, pins, nets, terminal order and keepouts.

use of_circuit::{CircuitFile, Dir, NetKind as FileNetKind, SchematicPin, parse};

use super::model::{Component, Direction, Net, NetKind, Pin, PinRef, Point, Rect, Schematic};

const BLINK: &str = include_str!("../../../../circuits/demos/blink.circuit.toml");
const DIMMER: &str = include_str!("../../../../circuits/demos/dimmer.circuit.toml");
const FADE: &str = include_str!("../../../../circuits/demos/fade.circuit.toml");
const BUTTON: &str = include_str!("../../../../circuits/demos/button.circuit.toml");
const BUTTON_IRQ: &str = include_str!("../../../../circuits/demos/button-irq.circuit.toml");

fn dir(d: Dir) -> Direction {
    match d {
        Dir::N => Direction::North,
        Dir::E => Direction::East,
        Dir::S => Direction::South,
        Dir::W => Direction::West,
    }
}

fn at(origin: [f32; 2], rel: [f32; 2]) -> Point {
    Point::new(origin[0] + rel[0], origin[1] + rel[1])
}

/// The net holding `reference.pin` (case-insensitive).
fn net_of(file: &CircuitFile, reference: &str, pin: &str) -> Option<String> {
    let want = format!("{reference}.{pin}").to_lowercase();
    file.nets
        .iter()
        .find(|n| n.pins().any(|p| p.to_lowercase() == want))
        .map(|n| n.name().to_owned())
}

fn pins(
    file: &CircuitFile,
    reference: &str,
    origin: [f32; 2],
    pins: &[SchematicPin],
    param: &dyn Fn(&str) -> f32,
) -> Vec<Pin> {
    pins.iter()
        .map(|p| {
            let mut rel = p.at;
            if let Some(s) = &p.slide {
                let k = param(s.param.get_ref());
                rel = [rel[0] + (s.to[0] - rel[0]) * k, rel[1] + (s.to[1] - rel[1]) * k];
            }
            Pin {
                id: p.id.clone().unwrap_or_else(|| p.pin.get_ref().clone()),
                name: p.name.clone().unwrap_or_else(|| p.pin.get_ref().clone()),
                position: at(origin, rel),
                direction: dir(p.dir),
                net: net_of(file, reference, p.pin.get_ref()),
                routed: !p.stub,
            }
        })
        .collect()
}

fn body(origin: [f32; 2], b: [f32; 4]) -> Rect {
    Rect::from_min_max(at(origin, [b[0], b[1]]), at(origin, [b[2], b[3]]))
}

/// The unrouted schematic a file describes.
fn schematic(file: &CircuitFile) -> Schematic {
    let mut components = Vec::new();
    let mut ids = std::collections::BTreeMap::new();
    for c in &file.components {
        let Some(s) = &c.schematic else { continue };
        let origin = c.at.unwrap_or([0.0, 0.0]);
        let id = s.id.clone().unwrap_or_else(|| c.reference().to_lowercase());
        ids.insert(c.reference().to_lowercase(), (id.clone(), s.pins.clone()));
        let param = |name: &str| c.params.get(name).and_then(|v| v.as_number()).map_or(0.5, |v| v as f32);
        components.push(Component {
            id,
            reference: s.label.clone().unwrap_or_else(|| c.reference().to_owned()),
            body: body(origin, s.body),
            pins: pins(file, c.reference(), origin, &s.pins, &param),
        });
    }
    for y in &file.symbols {
        let id = y.id.clone().unwrap_or_else(|| y.reference().to_lowercase());
        ids.insert(y.reference().to_lowercase(), (id.clone(), y.pins.clone()));
        components.push(Component {
            id,
            reference: y.reference().to_owned(),
            body: body(y.at, y.body),
            pins: pins(file, y.reference(), y.at, &y.pins, &|_| 0.5),
        });
    }
    let nets = file
        .nets
        .iter()
        .map(|n| Net {
            id: n.name().to_owned(),
            name: n.label.clone().unwrap_or_else(|| n.name().to_owned()),
            kind: match n.kind {
                FileNetKind::Signal => NetKind::Signal,
                FileNetKind::Power => NetKind::Power,
                FileNetKind::Ground => NetKind::Ground,
            },
            terminals: n
                .pins()
                .map(|t| {
                    let (r, p) = t.split_once('.').expect("REF.PIN");
                    let (id, spins) = &ids[&r.to_lowercase()];
                    let sp = spins
                        .iter()
                        .find(|s| s.pin.get_ref().eq_ignore_ascii_case(p))
                        .unwrap_or_else(|| panic!("{t} is not drawn"));
                    PinRef::new(id.clone(), sp.id.clone().unwrap_or_else(|| sp.pin.get_ref().clone()))
                })
                .collect(),
            waypoints: n.waypoints.iter().map(|w| Point::new(w[0], w[1])).collect(),
            route: None,
        })
        .collect();
    let keepouts = file
        .schematic
        .keepouts
        .iter()
        .map(|k| Rect::from_min_max(Point::new(k.rect[0], k.rect[1]), Point::new(k.rect[2], k.rect[3])))
        .collect();
    Schematic {
        components,
        nets,
        fixture_strokes: Vec::new(),
        keepouts,
    }
}

fn unrouted(mut s: Schematic) -> Schematic {
    for n in &mut s.nets {
        n.route = None;
    }
    s
}

fn assert_same(name: &str, text: &str, legacy: Schematic) {
    let file = parse(text).unwrap_or_else(|e| panic!("{name}: {e}"));
    let got = schematic(&file);
    let want = unrouted(legacy);
    assert_eq!(got.components.len(), want.components.len(), "{name}: component count");
    for (g, w) in got.components.iter().zip(&want.components) {
        assert_eq!(g, w, "{name}: component {}", w.id);
    }
    assert_eq!(got.nets, want.nets, "{name}: nets");
    assert_eq!(got.keepouts, want.keepouts, "{name}: keepouts");
    assert_eq!(got, want, "{name}");
}

#[test]
fn blink_file_names_match_the_legacy_board() {
    assert_same("blink", BLINK, super::blink::board());
}

#[test]
fn dimmer_file_names_match_the_legacy_board() {
    assert_same("dimmer", DIMMER, super::dimmer::board());
}

#[test]
fn fade_file_names_match_the_legacy_board() {
    assert_same("fade", FADE, super::dimmer::fade_board());
}

#[test]
fn button_files_name_match_the_legacy_board() {
    assert_same("button", BUTTON, super::button::board());
    assert_same("button-irq", BUTTON_IRQ, super::button::board());
}
