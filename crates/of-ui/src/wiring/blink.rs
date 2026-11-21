//! Hand-authored blink demo board: components, nets, and fixture routes.
//!
//! Geometry matches `crates/of-ui/src/blink/schematic.rs` as of the PR 1
//! snapshot. Painting must keep the historical strokes, colours and order.

use crate::wiring::model::{
    Component, Direction, FixtureStroke, Net, NetKind, Pin, PinRef, Point, Rect, Route, Schematic, StrokeStyle,
};

fn p(x: f32, y: f32) -> Point {
    Point::new(x, y)
}

fn pin(id: &str, name: &str, pos: Point, dir: Direction, net: &str) -> Pin {
    Pin {
        id: id.into(),
        name: name.into(),
        position: pos,
        direction: dir,
        net: Some(net.into()),
        routed: true,
    }
}

/// The LED blink schematic in model form, with today's hand-authored routes.
pub fn board() -> Schematic {
    let mut schematic = Schematic {
        components: vec![
            Component {
                id: "u1".into(),
                reference: "U1".into(),
                body: Rect::from_min_max(p(18.0, 49.0), p(120.0, 149.0)),
                pins: vec![
                    pin("vdd", "VDD", p(18.0, 64.0), Direction::West, "vdd"),
                    pin("gpio8", "GPIO8", p(120.0, 82.0), Direction::East, "gpio8"),
                    pin("gnd", "GND", p(120.0, 133.0), Direction::East, "gnd"),
                ],
            },
            Component {
                id: "r1".into(),
                reference: "R1".into(),
                // Hover/symbol bounds; the zigzag of the series resistor sits inside.
                body: Rect::from_min_max(p(157.0, 70.0), p(207.0, 94.0)),
                pins: vec![
                    pin("1", "1", p(157.0, 82.0), Direction::West, "gpio8"),
                    pin("2", "2", p(207.0, 82.0), Direction::East, "led_a"),
                ],
            },
            Component {
                id: "d1".into(),
                reference: "D1".into(),
                // Symbol triangle/bar, not the larger hover target used for tooltips.
                body: Rect::from_min_max(p(267.0, 70.0), p(291.0, 95.0)),
                pins: vec![
                    pin("A", "A", p(267.0, 82.0), Direction::West, "led_a"),
                    pin("K", "K", p(291.0, 82.0), Direction::East, "gnd"),
                ],
            },
            Component {
                id: "psu".into(),
                reference: "PSU".into(),
                body: Rect::from_min_max(p(18.0, 210.0), p(96.0, 262.0)),
                pins: vec![
                    pin("plus", "+", p(96.0, 224.0), Direction::East, "vdd"),
                    pin("minus", "-", p(96.0, 252.0), Direction::East, "gnd"),
                ],
            },
            Component {
                id: "gnd_sym".into(),
                reference: "GND".into(),
                body: Rect::from_min_max(p(211.0, 174.0), p(233.0, 184.0)),
                pins: vec![pin("pin", "1", p(222.0, 174.0), Direction::North, "gnd")],
            },
        ],
        nets: vec![
            net("gpio8", "GPIO8", NetKind::Signal, &[("u1", "gpio8"), ("r1", "1")]),
            net("led_a", "LED anode", NetKind::Signal, &[("r1", "2"), ("d1", "A")]),
            net("vdd", "VDD", NetKind::Power, &[("u1", "vdd"), ("psu", "plus")]),
            net(
                "gnd",
                "GND",
                NetKind::Ground,
                &[("u1", "gnd"), ("d1", "K"), ("psu", "minus"), ("gnd_sym", "pin")],
            ),
        ],
        fixture_strokes: vec![
            stroke(
                "gpio8_to_led_a",
                StrokeStyle::LiveSignal,
                &["gpio8", "led_a"],
                &[
                    p(120.0, 82.0),
                    p(157.0, 82.0),
                    p(162.0, 74.0),
                    p(170.0, 90.0),
                    p(178.0, 74.0),
                    p(186.0, 90.0),
                    p(194.0, 74.0),
                    p(202.0, 90.0),
                    p(207.0, 82.0),
                    p(267.0, 82.0),
                ],
                false,
            ),
            stroke(
                "gnd_return",
                StrokeStyle::Wire,
                &["gnd"],
                &[
                    p(291.0, 82.0),
                    p(382.0, 82.0),
                    p(382.0, 154.0),
                    p(142.0, 154.0),
                    p(142.0, 133.0),
                    p(120.0, 133.0),
                ],
                false,
            ),
            stroke(
                "gnd_symbol_lead",
                StrokeStyle::Wire,
                &["gnd"],
                &[p(222.0, 154.0), p(222.0, 174.0)],
                false,
            ),
            stroke(
                "vdd",
                StrokeStyle::Power,
                &["vdd"],
                &[p(96.0, 224.0), p(8.0, 224.0), p(8.0, 64.0), p(18.0, 64.0)],
                false,
            ),
            stroke(
                "psu_minus",
                StrokeStyle::Wire,
                &["gnd"],
                &[p(96.0, 252.0), p(96.0, 258.0)],
                true,
            ),
        ],
        keepouts: Vec::new(),
    };

    attach_routes(&mut schematic);
    schematic
}

fn net(id: &str, name: &str, kind: NetKind, terminals: &[(&str, &str)]) -> Net {
    Net {
        id: id.into(),
        name: name.into(),
        kind,
        terminals: terminals.iter().map(|(c, p)| PinRef::new(*c, *p)).collect(),
        waypoints: Vec::new(),
        route: None,
    }
}

fn stroke(id: &str, style: StrokeStyle, nets: &[&str], points: &[Point], line_segment: bool) -> FixtureStroke {
    FixtureStroke {
        id: id.into(),
        style,
        nets: nets.iter().map(|n| (*n).into()).collect(),
        points: points.to_vec(),
        line_segment,
    }
}

fn attach_routes(schematic: &mut Schematic) {
    let junction = p(222.0, 154.0);
    for net in &mut schematic.nets {
        let mut polylines = Vec::new();
        for stroke in &schematic.fixture_strokes {
            if stroke.nets.iter().any(|n| n == &net.id) {
                polylines.push(stroke.points.clone());
            }
        }
        let mut junctions = Vec::new();
        if net.id == "gnd" {
            junctions.push(junction);
        }
        net.route = Some(Route {
            polylines,
            junctions,
            hops: Vec::new(),
        });
    }
}
