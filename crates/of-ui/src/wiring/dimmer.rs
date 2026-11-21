//! Pot dimmer and LED fade boards: components, nets, and generated routes.
//!
//! The dimmer board adds RV1 and a right-hand PSU; the fade board is the same
//! LED/resistor circuit without the pot, with the PSU under the chip. The R2
//! zigzag is symbol geometry (not a net stroke). Label boxes are keepouts so
//! routes do not run through text.

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

fn stub_pin(id: &str, name: &str, pos: Point, dir: Direction, net: &str) -> Pin {
    Pin {
        routed: false,
        ..pin(id, name, pos, dir, net)
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

/// Canonical dimmer board (RV1 at mid-travel, the desktop slider default).
pub fn board() -> Schematic {
    board_at(0.5)
}

/// Dimmer board with the pot wiper at `position` in 0..=1 (0 = pin 1 / bottom).
pub fn board_at(position: f64) -> Schematic {
    build(Some(position.clamp(0.0, 1.0) as f32))
}

/// LED fade board: same LED net as the dimmer, no pot, PSU under the chip.
pub fn fade_board() -> Schematic {
    build(None)
}

fn build(wiper: Option<f32>) -> Schematic {
    let mut u1_pins = vec![
        pin("vdd", "VDD", p(18.0, 64.0), Direction::West, "vdd"),
        pin("gpio8", "GPIO8", p(120.0, 82.0), Direction::East, "gpio8"),
        pin("gnd", "GND", p(120.0, 133.0), Direction::East, "gnd"),
    ];
    if wiper.is_some() {
        u1_pins.insert(2, pin("gpio2", "GPIO2", p(120.0, 112.0), Direction::East, "gpio2"));
    }

    // The painted chip is always (18,49)-(120,149). On the dimmer, inset the
    // routing body 4 px so the VDD and GND rails (hand-authored 4 px apart)
    // both fit under the chip with clearance 4. The fade board has no pot rail
    // and keeps the full body, matching blink.
    let u1_body = if wiper.is_some() {
        Rect::from_min_max(p(18.0, 49.0), p(120.0, 145.0))
    } else {
        Rect::from_min_max(p(18.0, 49.0), p(120.0, 149.0))
    };

    let mut components = vec![
        Component {
            id: "u1".into(),
            reference: "U1".into(),
            body: u1_body,
            pins: u1_pins,
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
    ];

    if let Some(pos) = wiper {
        // Track body only; the teal wiper bar is symbol geometry.
        let wy = 228.0 - 60.0 * pos;
        components.push(Component {
            id: "rv1".into(),
            reference: "RV1".into(),
            body: Rect::from_min_max(p(52.0, 168.0), p(72.0, 228.0)),
            pins: vec![
                pin("3", "3", p(62.0, 168.0), Direction::North, "vdd"),
                stub_pin("1", "1", p(62.0, 228.0), Direction::South, "gnd"),
                pin("wiper", "wiper", p(86.0, wy), Direction::East, "gpio2"),
            ],
        });
        // Right-hand PSU, plus on the left edge.
        components.push(Component {
            id: "psu".into(),
            reference: "PSU".into(),
            body: Rect::from_min_max(p(300.0, 168.0), p(378.0, 220.0)),
            pins: vec![
                pin("plus", "+", p(300.0, 182.0), Direction::West, "vdd"),
                // No visible minus stub or label on this board; name connectivity only.
                stub_pin("minus", "-", p(300.0, 210.0), Direction::West, "gnd"),
            ],
        });
    } else {
        components.push(Component {
            id: "psu".into(),
            reference: "PSU".into(),
            body: Rect::from_min_max(p(18.0, 210.0), p(96.0, 262.0)),
            pins: vec![
                pin("plus", "+", p(96.0, 224.0), Direction::East, "vdd"),
                stub_pin("minus", "-", p(96.0, 252.0), Direction::East, "gnd"),
            ],
        });
    }

    components.push(Component {
        id: "gnd_sym".into(),
        reference: "GND".into(),
        body: Rect::from_min_max(p(211.0, 174.0), p(233.0, 184.0)),
        pins: vec![pin("pin", "1", p(222.0, 174.0), Direction::North, "gnd")],
    });

    let mut nets = vec![
        net("gpio8", "GPIO8", NetKind::Signal, &[("u1", "gpio8"), ("r2", "1")]),
        net("led_a", "LED anode", NetKind::Signal, &[("r2", "2"), ("d1", "A")]),
        net(
            "vdd",
            "VDD",
            NetKind::Power,
            if wiper.is_some() {
                &[("u1", "vdd"), ("psu", "plus"), ("rv1", "3")][..]
            } else {
                &[("u1", "vdd"), ("psu", "plus")][..]
            },
        ),
        net(
            "gnd",
            "GND",
            NetKind::Ground,
            if wiper.is_some() {
                &[
                    ("u1", "gnd"),
                    ("d1", "K"),
                    ("psu", "minus"),
                    ("gnd_sym", "pin"),
                    ("rv1", "1"),
                ][..]
            } else {
                &[("u1", "gnd"), ("d1", "K"), ("psu", "minus"), ("gnd_sym", "pin")][..]
            },
        ),
    ];
    if wiper.is_some() {
        nets.push(net(
            "gpio2",
            "GPIO2",
            NetKind::Signal,
            &[("u1", "gpio2"), ("rv1", "wiper")],
        ));
    }

    let mut keepouts = vec![
        Rect::from_min_max(p(18.0, 28.0), p(112.0, 40.0)),    // U1 / ESP32-C3
        Rect::from_min_max(p(168.0, 28.0), p(196.0, 40.0)),   // R2
        Rect::from_min_max(p(160.0, 47.0), p(204.0, 59.0)),   // 330 ohm
        Rect::from_min_max(p(264.0, 28.0), p(288.0, 40.0)),   // D1
        Rect::from_min_max(p(251.0, 100.0), p(265.0, 112.0)), // A
        Rect::from_min_max(p(294.0, 100.0), p(308.0, 112.0)), // K
        Rect::from_min_max(p(254.0, 119.0), p(304.0, 131.0)), // Red LED
        Rect::from_min_max(p(211.0, 195.0), p(233.0, 207.0)), // GND under symbol
    ];
    if wiper.is_some() {
        keepouts.extend([
            Rect::from_min_max(p(96.0, 232.0), p(120.0, 244.0)), // RV1
            Rect::from_min_max(p(98.0, 162.0), p(140.0, 174.0)), // 10 k ohm
            Rect::from_min_max(p(36.0, 172.0), p(50.0, 184.0)),  // pin 3
            Rect::from_min_max(p(36.0, 214.0), p(50.0, 226.0)),  // pin 1
            Rect::from_min_max(p(50.0, 252.0), p(74.0, 264.0)),  // GND under pot
            // Block the sky above the chip so the GND return stays under the
            // board instead of looping over the labels at y=44.
            Rect::from_min_max(p(8.0, 16.0), p(400.0, 50.0)),
        ]);
    } else {
        keepouts.push(Rect::from_min_max(p(85.0, 260.0), p(107.0, 272.0))); // GND under PSU
    }

    let mut schematic = Schematic {
        components,
        nets,
        fixture_strokes: Vec::new(),
        keepouts,
    };
    router::route(&mut schematic);
    schematic
}
