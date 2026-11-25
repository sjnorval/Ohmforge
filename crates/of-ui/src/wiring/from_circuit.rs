//! The schematic a circuit file describes (engine E4c).
//!
//! A file carries each component's layout in `[component.schematic]` (body,
//! pins, the board's ids and labels) and the nets in router order, so
//! [`from_circuit`] yields the [`Schematic`] the route goldens pin (the
//! hand-written boards it replaced in E4d drew the same):
//! components in file order (symbols after them), pins with their
//! drawn ids and names, nets with their terminals in file order, waypoints
//! and keepouts. [`super::route`] then routes it. Live parameters (the pot
//! wiper) come from a closure so the dimmer's wiper pin follows the slider.

use std::collections::BTreeMap;

use of_circuit::{CircuitFile, Dir, NetKind as FileNetKind, SchematicPin};

use super::model::{Component, Direction, Net, NetKind, Pin, PinRef, Point, Rect, Schematic};
use super::router;

/// Live parameter lookup: `(REF, param)` to a value, or `None` to use the
/// file's own value.
pub type ParamLookup<'a> = &'a dyn Fn(&str, &str) -> Option<f64>;

/// No live parameters: every value comes from the file.
pub fn file_params(_reference: &str, _param: &str) -> Option<f64> {
    None
}

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
            let mut position = at(origin, p.at);
            if let Some(s) = &p.slide {
                // In absolute coordinates, as the hand-written boards compute it.
                let k = param(s.param.get_ref()).clamp(0.0, 1.0);
                let to = at(origin, s.to);
                position = Point::new(
                    position.x + (to.x - position.x) * k,
                    position.y + (to.y - position.y) * k,
                );
            }
            Pin {
                id: p.id.clone().unwrap_or_else(|| p.pin.get_ref().clone()),
                name: p.name.clone().unwrap_or_else(|| p.pin.get_ref().clone()),
                position,
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

/// The schematic `file` describes, before routing. Components without a
/// `[component.schematic]` block are not drawn. A net terminal on a pin the
/// file does not draw is an error.
pub fn unrouted(file: &CircuitFile, live: ParamLookup<'_>) -> Result<Schematic, String> {
    let mut components = Vec::new();
    let mut ids = BTreeMap::new();
    for c in &file.components {
        let Some(s) = &c.schematic else { continue };
        let origin = c.at.unwrap_or([0.0, 0.0]);
        let id = s.id.clone().unwrap_or_else(|| c.reference().to_lowercase());
        ids.insert(c.reference().to_lowercase(), (id.clone(), s.pins.clone()));
        let param = |name: &str| {
            live(c.reference(), name)
                .or_else(|| c.params.get(name).and_then(|v| v.as_number()))
                .map_or(0.5, |v| v as f32)
        };
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
    let mut nets = Vec::with_capacity(file.nets.len());
    for n in &file.nets {
        let mut terminals = Vec::new();
        for t in n.pins() {
            let (r, p) = t
                .split_once('.')
                .ok_or_else(|| format!("net {}: {t} is not REF.PIN", n.name()))?;
            let (id, spins) = ids
                .get(&r.to_lowercase())
                .ok_or_else(|| format!("net {}: {r} is not drawn", n.name()))?;
            let sp = spins
                .iter()
                .find(|s| s.pin.get_ref().eq_ignore_ascii_case(p))
                .ok_or_else(|| format!("net {}: {t} is not drawn", n.name()))?;
            terminals.push(PinRef::new(
                id.clone(),
                sp.id.clone().unwrap_or_else(|| sp.pin.get_ref().clone()),
            ));
        }
        nets.push(Net {
            id: n.name().to_owned(),
            name: n.label.clone().unwrap_or_else(|| n.name().to_owned()),
            kind: match n.kind {
                FileNetKind::Signal => NetKind::Signal,
                FileNetKind::Power => NetKind::Power,
                FileNetKind::Ground => NetKind::Ground,
            },
            terminals,
            waypoints: n.waypoints.iter().map(|w| Point::new(w[0], w[1])).collect(),
            route: None,
        });
    }
    let keepouts = file
        .schematic
        .keepouts
        .iter()
        .map(|k| Rect::from_min_max(Point::new(k.rect[0], k.rect[1]), Point::new(k.rect[2], k.rect[3])))
        .collect();
    Ok(Schematic {
        components,
        nets,
        fixture_strokes: Vec::new(),
        keepouts,
    })
}

/// The routed schematic `file` describes.
pub fn from_circuit(file: &CircuitFile, live: ParamLookup<'_>) -> Result<Schematic, String> {
    let mut s = unrouted(file, live)?;
    router::route(&mut s);
    Ok(s)
}

#[cfg(test)]
mod tests {
    use of_circuit::parse;

    use super::*;
    use crate::wiring::{check, serialize};

    use crate::wiring::demos::{BLINK, BUTTON, BUTTON_IRQ, DIMMER, FADE};

    fn file(name: &str, text: &str) -> CircuitFile {
        parse(text).unwrap_or_else(|e| panic!("{name}: {e}"))
    }

    /// Routed, the file's schematic serialises byte for byte to the route
    /// golden, and the router invariants hold.
    fn assert_routes(name: &str, routed: &Schematic, golden: &str) {
        assert_eq!(
            serialize(routed),
            golden,
            "{name}: routes differ from goldens/{name}.routes.txt"
        );
        let v = check(routed);
        assert!(
            v.is_empty(),
            "{name} invariants:\n{}",
            v.iter().map(|v| v.line()).collect::<Vec<_>>().join("\n")
        );
    }

    #[test]
    fn blink_file_routes_match_the_golden() {
        let f = file("blink", BLINK);
        assert_routes(
            "blink",
            &from_circuit(&f, &file_params).unwrap(),
            include_str!("goldens/blink.routes.txt"),
        );
    }

    /// The wiper pin follows the live position (clamped to 0..=1), the router
    /// invariants hold at every position, and mid-travel is the golden.
    #[test]
    fn dimmer_file_routes_match_the_golden_at_every_wiper_position() {
        let f = file("dimmer", DIMMER);
        assert_routes(
            "dimmer",
            &from_circuit(&f, &file_params).unwrap(),
            include_str!("goldens/dimmer.routes.txt"),
        );
        let at = |pos: f64| {
            let live = move |r: &str, p: &str| (r == "RV1" && p == "position").then_some(pos);
            from_circuit(&f, &live).unwrap()
        };
        let wiper_y = |s: &Schematic| {
            let rv1 = s.components.iter().find(|c| c.reference == "RV1").expect("RV1");
            // Pin ids are the drawn ids (`wiper`), not the part's pin names.
            rv1.pins.iter().find(|p| p.id == "wiper").expect("wiper").position.y
        };
        assert_eq!(serialize(&at(0.5)), include_str!("goldens/dimmer.routes.txt"));
        assert_eq!(serialize(&at(-0.5)), serialize(&at(0.0)), "clamped below");
        assert_eq!(serialize(&at(1.5)), serialize(&at(1.0)), "clamped above");
        let mut last = f32::INFINITY;
        for pos in [0.0, 0.1, 0.25, 1.0 / 3.0, 0.5, 0.75, 0.9, 1.0] {
            let s = at(pos);
            let v = check(&s);
            assert!(v.is_empty(), "dimmer at {pos}: {v:?}");
            // 0 is pin 1 at the bottom; the wiper climbs as the position rises.
            let y = wiper_y(&s);
            assert!(y < last || pos == 0.0, "wiper at {pos}: y {y} after {last}");
            last = y;
        }
    }

    #[test]
    fn fade_file_routes_match_the_golden() {
        let f = file("fade", FADE);
        assert_routes(
            "fade",
            &from_circuit(&f, &file_params).unwrap(),
            include_str!("goldens/fade.routes.txt"),
        );
    }

    #[test]
    fn button_files_route_like_the_golden() {
        for (name, text) in [("button", BUTTON), ("button-irq", BUTTON_IRQ)] {
            let f = file(name, text);
            assert_routes(
                "button",
                &from_circuit(&f, &file_params).unwrap(),
                include_str!("goldens/button.routes.txt"),
            );
        }
    }

    #[test]
    fn a_terminal_on_an_undrawn_component_is_an_error() {
        let mut f = file("blink", BLINK);
        let d1 = f.components.iter_mut().find(|c| c.reference() == "D1").unwrap();
        d1.schematic = None;
        let e = unrouted(&f, &file_params).unwrap_err();
        assert!(e.contains("D1 is not drawn"), "{e}");
    }
}
