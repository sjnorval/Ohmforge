//! Blink demo board: components, nets, symbol geometry, and generated routes.
//!
//! The resistor zigzag is symbol geometry (not a net stroke). Net paths are
//! produced by the orthogonal grid router. Label boxes are keepouts so routes
//! do not run through text.

use crate::wiring::model::{Component, Direction, Net, NetKind, Pin, PinRef, Point, Rect, Schematic};
use crate::wiring::router;

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

/// The series-resistor zigzag, in schematic units, from R1 pin 1 to pin 2.
///
/// This is part of the R1 symbol and is painted with the live signal stroke so
/// the zig-zag still lights with the GPIO/LED net, but it is not a routed net
/// segment and must not be treated as a body crossing.
pub fn r1_zigzag() -> Vec<Point> {
    vec![
        p(157.0, 82.0),
        p(162.0, 74.0),
        p(170.0, 90.0),
        p(178.0, 74.0),
        p(186.0, 90.0),
        p(194.0, 74.0),
        p(202.0, 90.0),
        p(207.0, 82.0),
    ]
}

/// The LED blink schematic in model form, with generated orthogonal routes.
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
                    pin("minus", "\u{2212}", p(96.0, 252.0), Direction::East, "gnd"),
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
        fixture_strokes: Vec::new(),
        // Approximate 12 px monospace label boxes so routes stay out of text.
        keepouts: vec![
            Rect::from_min_max(p(18.0, 28.0), p(112.0, 40.0)),    // U1 / ESP32-C3
            Rect::from_min_max(p(168.0, 28.0), p(196.0, 40.0)),   // R1
            Rect::from_min_max(p(160.0, 47.0), p(204.0, 59.0)),   // 330 Ω
            Rect::from_min_max(p(264.0, 28.0), p(288.0, 40.0)),   // D1
            Rect::from_min_max(p(251.0, 100.0), p(265.0, 112.0)), // A
            Rect::from_min_max(p(294.0, 100.0), p(308.0, 112.0)), // K
            Rect::from_min_max(p(254.0, 119.0), p(304.0, 131.0)), // Red LED
            Rect::from_min_max(p(211.0, 195.0), p(233.0, 207.0)), // GND under symbol
            Rect::from_min_max(p(104.0, 256.0), p(128.0, 268.0)), // GND right of the PSU, under its return
        ],
    };

    router::route(&mut schematic);
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
