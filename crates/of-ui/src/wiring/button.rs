//! Push-button demo board: components, nets, and generated routes.
//!
//! SW1 pulls GPIO9 to ground. The R2 zigzag is symbol geometry (not a net
//! stroke). Label boxes are keepouts so routes do not run through text.

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

/// The push-button schematic in model form, with generated orthogonal routes.
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
                    pin("gpio9", "GPIO9", p(120.0, 112.0), Direction::East, "gpio9"),
                    pin("gnd", "GND", p(120.0, 133.0), Direction::East, "gnd"),
                ],
            },
            Component {
                id: "r2".into(),
                reference: "R2".into(),
                body: Rect::from_min_max(p(157.0, 70.0), p(207.0, 94.0)),
                pins: vec![
                    pin("1", "1", p(157.0, 82.0), Direction::West, "gpio8"),
                    pin("2", "2", p(207.0, 82.0), Direction::East, "led_a"),
                ],
            },
            Component {
                id: "d1".into(),
                reference: "D1".into(),
                body: Rect::from_min_max(p(267.0, 70.0), p(291.0, 95.0)),
                pins: vec![
                    pin("A", "A", p(267.0, 82.0), Direction::West, "led_a"),
                    pin("K", "K", p(291.0, 82.0), Direction::East, "gnd"),
                ],
            },
            Component {
                id: "sw1".into(),
                reference: "SW1".into(),
                // Painted switch stays (130,168)-(186,204). Left inset so a street at
                // x=128 exists; the hand trunk at x=132 entered the symbol by 2 px.
                body: Rect::from_min_max(p(136.0, 168.0), p(186.0, 204.0)),
                pins: vec![
                    // Left edge: GPIO side. The hand-authored trunk at x=132 entered
                    // the body by 2 px; the pin faces west so the stub stays outside.
                    pin("a", "GPIO9", p(130.0, 190.0), Direction::North, "gpio9"),
                    pin("b", "GND", p(186.0, 190.0), Direction::East, "gnd"),
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
            net("gpio8", "GPIO8", NetKind::Signal, &[("u1", "gpio8"), ("r2", "1")]),
            net("led_a", "LED anode", NetKind::Signal, &[("r2", "2"), ("d1", "A")]),
            net("gpio9", "GPIO9", NetKind::Signal, &[("u1", "gpio9"), ("sw1", "a")]),
            net("vdd", "VDD", NetKind::Power, &[("u1", "vdd"), ("psu", "plus")]),
            net(
                "gnd",
                "GND",
                NetKind::Ground,
                &[
                    ("u1", "gnd"),
                    ("d1", "K"),
                    ("psu", "minus"),
                    ("gnd_sym", "pin"),
                    ("sw1", "b"),
                ],
            ),
        ],
        fixture_strokes: Vec::new(),
        keepouts: vec![
            Rect::from_min_max(p(18.0, 28.0), p(112.0, 40.0)),    // U1 / ESP32-C3
            Rect::from_min_max(p(168.0, 28.0), p(196.0, 40.0)),   // R2
            Rect::from_min_max(p(160.0, 47.0), p(204.0, 59.0)),   // 330 ohm
            Rect::from_min_max(p(264.0, 28.0), p(288.0, 40.0)),   // D1
            Rect::from_min_max(p(251.0, 100.0), p(265.0, 112.0)), // A
            Rect::from_min_max(p(294.0, 100.0), p(308.0, 112.0)), // K
            Rect::from_min_max(p(254.0, 119.0), p(304.0, 131.0)), // Red LED
            Rect::from_min_max(p(211.0, 195.0), p(233.0, 207.0)), // GND under symbol
            Rect::from_min_max(p(104.0, 256.0), p(128.0, 268.0)), // GND right of the PSU, under its return
            Rect::from_min_max(p(144.0, 166.0), p(172.0, 178.0)), // SW1 label
        ],
    };
    router::route(&mut schematic);
    schematic
}
