//! Schematic wiring model, fixture routes, and geometry invariants.
//!
//! This module is pure Rust and is compiled without the `gui` feature so the
//! goldens and invariants run in the workspace test job.

mod blink;
mod button;
mod dimmer;
mod golden;
mod invariants;
mod model;
mod router;

pub use model::{
    CLEARANCE, Component, Direction, FixtureStroke, GRID, Hop, Net, NetKind, Pin, PinRef, Point, Rect, Route,
    Schematic, StrokeStyle,
};

pub use blink::{board as blink_board, r1_zigzag};
pub use button::board as button_board;
pub use dimmer::{board as dimmer_board, board_at as dimmer_board_at, fade_board};
pub use golden::serialize;
pub use invariants::check;
pub use router::route;

#[cfg(test)]
mod tests {
    use super::{
        Hop, Point, StrokeStyle, blink_board, button_board, check, dimmer_board, dimmer_board_at, fade_board, serialize,
    };

    const BLINK_GOLDEN: &str = include_str!("goldens/blink.routes.txt");
    const BLINK_INVARIANTS: &str = include_str!("goldens/blink.invariants.txt");
    const DIMMER_GOLDEN: &str = include_str!("goldens/dimmer.routes.txt");
    const DIMMER_INVARIANTS: &str = include_str!("goldens/dimmer.invariants.txt");
    const FADE_GOLDEN: &str = include_str!("goldens/fade.routes.txt");
    const FADE_INVARIANTS: &str = include_str!("goldens/fade.invariants.txt");
    const BUTTON_GOLDEN: &str = include_str!("goldens/button.routes.txt");
    const BUTTON_INVARIANTS: &str = include_str!("goldens/button.invariants.txt");

    #[test]
    fn blink_serialise_is_deterministic() {
        let board = blink_board();
        assert!(board.component("u1").is_some());
        assert!(board.net("gnd").is_some());
        assert!(board.net("vdd").and_then(|n| n.route.as_ref()).is_some());
        assert!(board.stroke("vdd").is_none());
        let a = serialize(&board);
        let b = serialize(&blink_board());
        assert_eq!(a, b);
    }

    #[test]
    fn blink_fixture_matches_golden() {
        let got = serialize(&blink_board());
        assert_eq!(
            got, BLINK_GOLDEN,
            "blink route golden drifted; if the change is intended, update crates/of-ui/src/wiring/goldens/blink.routes.txt\n--- got ---\n{got}"
        );
    }

    #[test]
    fn blink_invariants_hold() {
        // Generated blink routes must satisfy every router convention: no body
        // crossings, no foreign-net overlap, degree-3 junctions only, and
        // perpendicular stubs of at least one grid step.
        let violations = check(&blink_board());
        assert!(
            violations.is_empty(),
            "blink invariants failed:\n{}",
            violations.iter().map(|v| v.line()).collect::<Vec<_>>().join("\n")
        );
        let got = check(&blink_board())
            .iter()
            .map(|v| v.line())
            .collect::<Vec<_>>()
            .join("\n");
        let got = if got.is_empty() { got } else { format!("{got}\n") };
        assert_eq!(
            got, BLINK_INVARIANTS,
            "blink invariant snapshot drifted; update crates/of-ui/src/wiring/goldens/blink.invariants.txt\n--- got ---\n{got}"
        );
    }

    #[test]
    fn blink_router_is_deterministic() {
        let a = serialize(&blink_board());
        let b = serialize(&blink_board());
        assert_eq!(a, b);
    }

    #[test]
    fn r1_zigzag_is_not_a_net_route() {
        let board = blink_board();
        let zigzag = super::r1_zigzag();
        for net in &board.nets {
            let Some(route) = &net.route else { continue };
            for poly in &route.polylines {
                for w in poly.windows(2) {
                    for z in zigzag.windows(2) {
                        // The zig-zag interior segments must not appear on a net.
                        let interior = z[0] != *zigzag.first().unwrap() && z[1] != *zigzag.last().unwrap()
                            || (z[0] != *zigzag.first().unwrap() && z[0] != *zigzag.last().unwrap());
                        if !interior {
                            continue;
                        }
                        if w[0].distance(z[0]) <= 1e-4 && w[1].distance(z[1]) <= 1e-4
                            || w[0].distance(z[1]) <= 1e-4 && w[1].distance(z[0]) <= 1e-4
                        {
                            panic!("net {} reused R1 zigzag segment {}–{}", net.id, z[0].x, z[1].x);
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn hop_round_trip_fields_are_preserved() {
        // Hops are unused on the blink fixture; keep the type live and comparable.
        let hop = Hop {
            over: Point::new(10.0, 20.0),
            across_horizontal: true,
        };
        assert_eq!(hop.over, Point::new(10.0, 20.0));
        assert!(hop.across_horizontal);
        assert_eq!(StrokeStyle::LiveSignal.as_str(), "live");
        assert_eq!(StrokeStyle::Power.as_str(), "power");
        assert_eq!(StrokeStyle::Wire.as_str(), "wire");
        assert_eq!(super::Direction::South.as_str(), "S");
        assert_eq!(super::Direction::South.vector(), (0.0, 1.0));
    }

    #[test]
    fn a_clean_manhattan_stub_has_no_violations() {
        use super::{
            Component, Direction, FixtureStroke, Net, NetKind, Pin, PinRef, Rect, Route, Schematic, StrokeStyle,
        };
        let schematic = Schematic {
            components: vec![
                Component {
                    id: "a".into(),
                    reference: "A".into(),
                    body: Rect::from_min_max(Point::new(0.0, 0.0), Point::new(20.0, 20.0)),
                    pins: vec![Pin {
                        id: "p".into(),
                        name: "P".into(),
                        position: Point::new(20.0, 10.0),
                        direction: Direction::East,
                        net: Some("n".into()),
                        routed: true,
                    }],
                },
                Component {
                    id: "b".into(),
                    reference: "B".into(),
                    body: Rect::from_min_max(Point::new(80.0, 0.0), Point::new(100.0, 20.0)),
                    pins: vec![Pin {
                        id: "p".into(),
                        name: "P".into(),
                        position: Point::new(80.0, 10.0),
                        direction: Direction::West,
                        net: Some("n".into()),
                        routed: true,
                    }],
                },
            ],
            nets: vec![Net {
                id: "n".into(),
                name: "n".into(),
                kind: NetKind::Signal,
                terminals: vec![PinRef::new("a", "p"), PinRef::new("b", "p")],
                waypoints: Vec::new(),
                route: Some(Route {
                    polylines: vec![vec![Point::new(20.0, 10.0), Point::new(80.0, 10.0)]],
                    junctions: Vec::new(),
                    hops: Vec::new(),
                }),
            }],
            fixture_strokes: vec![FixtureStroke {
                id: "n".into(),
                style: StrokeStyle::Wire,
                nets: vec!["n".into()],
                points: vec![Point::new(20.0, 10.0), Point::new(80.0, 10.0)],
                line_segment: false,
            }],
            keepouts: Vec::new(),
        };
        assert_eq!(check(&schematic), Vec::new(), "{:?}", check(&schematic));

        // The same two-pin board is also routed from scratch with no violations.
        let mut routed = schematic;
        routed.fixture_strokes.clear();
        routed.nets[0].route = None;
        super::route(&mut routed);
        assert!(routed.nets[0].route.is_some(), "expected a generated route");
        assert_eq!(check(&routed), Vec::new(), "{:?}", check(&routed));
    }

    #[test]
    fn net_order_is_power_ground_then_fanout_then_name() {
        // A tiny board with four nets: the sort key is kind, then fanout, then id.
        use super::{Component, Direction, Net, NetKind, Pin, PinRef, Point, Rect, Schematic};
        let pin = |id: &str, x, y, dir, net: &str| Pin {
            id: id.into(),
            name: id.into(),
            position: Point::new(x, y),
            direction: dir,
            net: Some(net.into()),
            routed: true,
        };
        // Place four 16x16 bodies in a row so every net can run a street along y=24.
        let mut schematic = Schematic {
            components: vec![
                Component {
                    id: "c0".into(),
                    reference: "C0".into(),
                    body: Rect::from_min_max(Point::new(0.0, 0.0), Point::new(16.0, 16.0)),
                    pins: vec![
                        pin("p", 16.0, 8.0, Direction::East, "pwr"),
                        pin("g", 16.0, 12.0, Direction::East, "gnd"),
                    ],
                },
                Component {
                    id: "c1".into(),
                    reference: "C1".into(),
                    body: Rect::from_min_max(Point::new(80.0, 0.0), Point::new(96.0, 16.0)),
                    pins: vec![
                        pin("p", 80.0, 8.0, Direction::West, "pwr"),
                        pin("g", 80.0, 12.0, Direction::West, "gnd"),
                        pin("a", 80.0, 4.0, Direction::West, "sig_a"),
                    ],
                },
                Component {
                    id: "c2".into(),
                    reference: "C2".into(),
                    body: Rect::from_min_max(Point::new(160.0, 0.0), Point::new(176.0, 16.0)),
                    pins: vec![
                        pin("g", 160.0, 12.0, Direction::West, "gnd"),
                        pin("a", 160.0, 4.0, Direction::West, "sig_a"),
                        pin("b", 176.0, 8.0, Direction::East, "sig_b"),
                    ],
                },
                Component {
                    id: "c3".into(),
                    reference: "C3".into(),
                    body: Rect::from_min_max(Point::new(240.0, 0.0), Point::new(256.0, 16.0)),
                    pins: vec![pin("b", 240.0, 8.0, Direction::West, "sig_b")],
                },
            ],
            nets: vec![
                Net {
                    id: "sig_b".into(),
                    name: "sig_b".into(),
                    kind: NetKind::Signal,
                    terminals: vec![PinRef::new("c2", "b"), PinRef::new("c3", "b")],
                    waypoints: Vec::new(),
                    route: None,
                },
                Net {
                    id: "sig_a".into(),
                    name: "sig_a".into(),
                    kind: NetKind::Signal,
                    terminals: vec![PinRef::new("c1", "a"), PinRef::new("c2", "a")],
                    waypoints: Vec::new(),
                    route: None,
                },
                Net {
                    id: "gnd".into(),
                    name: "gnd".into(),
                    kind: NetKind::Ground,
                    terminals: vec![PinRef::new("c0", "g"), PinRef::new("c1", "g"), PinRef::new("c2", "g")],
                    waypoints: Vec::new(),
                    route: None,
                },
                Net {
                    id: "pwr".into(),
                    name: "pwr".into(),
                    kind: NetKind::Power,
                    terminals: vec![PinRef::new("c0", "p"), PinRef::new("c1", "p")],
                    waypoints: Vec::new(),
                    route: None,
                },
            ],
            fixture_strokes: Vec::new(),
            keepouts: Vec::new(),
        };
        let order = super::router::net_order(&schematic);
        assert_eq!(order, ["pwr", "gnd", "sig_a", "sig_b"]);
        super::route(&mut schematic);
        for net in &schematic.nets {
            assert!(
                net.route.as_ref().is_some_and(|r| !r.polylines.is_empty()),
                "net {} unrouted",
                net.id
            );
        }
        assert_eq!(check(&schematic), Vec::new(), "{:?}", check(&schematic));
    }

    fn assert_invariants_hold(name: &str, board: &super::Schematic, snapshot: &str, snapshot_path: &str) {
        let violations = check(board);
        assert!(
            violations.is_empty(),
            "{name} invariants failed:\n{}",
            violations.iter().map(|v| v.line()).collect::<Vec<_>>().join("\n")
        );
        let got = violations.iter().map(|v| v.line()).collect::<Vec<_>>().join("\n");
        let got = if got.is_empty() { got } else { format!("{got}\n") };
        assert_eq!(
            got, snapshot,
            "{name} invariant snapshot drifted; update {snapshot_path}\n--- got ---\n{got}"
        );
    }

    fn maybe_write_golden(path: &str, body: &str) {
        if std::env::var("OF_WRITE_WIRING_GOLDENS").is_ok() {
            let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
            std::fs::write(root.join(path), body).expect("write golden");
        }
    }

    #[test]
    fn dimmer_serialise_is_deterministic() {
        let board = dimmer_board();
        assert!(board.component("u1").is_some());
        assert!(board.component("rv1").is_some());
        assert!(board.net("gpio2").and_then(|n| n.route.as_ref()).is_some());
        assert!(board.net("vdd").and_then(|n| n.route.as_ref()).is_some());
        let a = serialize(&board);
        let b = serialize(&dimmer_board());
        assert_eq!(a, b);
    }

    #[test]
    fn dimmer_routes_match_golden() {
        let got = serialize(&dimmer_board());
        maybe_write_golden("src/wiring/goldens/dimmer.routes.txt", &got);
        assert_eq!(
            got, DIMMER_GOLDEN,
            "dimmer route golden drifted; if the change is intended, update crates/of-ui/src/wiring/goldens/dimmer.routes.txt\n--- got ---\n{got}"
        );
    }

    #[test]
    fn dimmer_invariants_hold() {
        let board = dimmer_board();
        let got = check(&board).iter().map(|v| v.line()).collect::<Vec<_>>().join("\n");
        let got = if got.is_empty() { got } else { format!("{got}\n") };
        maybe_write_golden("src/wiring/goldens/dimmer.invariants.txt", &got);
        assert_invariants_hold(
            "dimmer",
            &board,
            DIMMER_INVARIANTS,
            "crates/of-ui/src/wiring/goldens/dimmer.invariants.txt",
        );
    }

    #[test]
    fn dimmer_router_is_deterministic() {
        assert_eq!(serialize(&dimmer_board()), serialize(&dimmer_board()));
    }

    #[test]
    fn dimmer_board_at_mid_matches_canonical_and_invariants_hold_at_ends() {
        assert_eq!(serialize(&dimmer_board()), serialize(&dimmer_board_at(0.5)));
        for pos in [0.0, 1.0] {
            let board = dimmer_board_at(pos);
            let v = check(&board);
            assert!(
                v.is_empty(),
                "dimmer at {pos} invariants failed:
{}",
                v.iter().map(|x| x.line()).collect::<Vec<_>>().join(
                    "
"
                )
            );
            assert!(board.net("gpio2").and_then(|n| n.route.as_ref()).is_some());
        }
    }

    #[test]
    fn fade_routes_match_golden() {
        let board = fade_board();
        assert!(board.component("rv1").is_none());
        assert!(board.net("gpio2").is_none());
        let got = serialize(&board);
        maybe_write_golden("src/wiring/goldens/fade.routes.txt", &got);
        assert_eq!(
            got, FADE_GOLDEN,
            "fade route golden drifted; if the change is intended, update crates/of-ui/src/wiring/goldens/fade.routes.txt\n--- got ---\n{got}"
        );
    }

    #[test]
    fn fade_invariants_hold() {
        let board = fade_board();
        let got = check(&board).iter().map(|v| v.line()).collect::<Vec<_>>().join("\n");
        let got = if got.is_empty() { got } else { format!("{got}\n") };
        maybe_write_golden("src/wiring/goldens/fade.invariants.txt", &got);
        assert_invariants_hold(
            "fade",
            &board,
            FADE_INVARIANTS,
            "crates/of-ui/src/wiring/goldens/fade.invariants.txt",
        );
    }

    #[test]
    fn fade_router_is_deterministic() {
        assert_eq!(serialize(&fade_board()), serialize(&fade_board()));
    }

    #[test]
    fn button_serialise_is_deterministic() {
        let board = button_board();
        assert!(board.component("sw1").is_some());
        assert!(board.net("gpio9").and_then(|n| n.route.as_ref()).is_some());
        let a = serialize(&board);
        let b = serialize(&button_board());
        assert_eq!(a, b);
    }

    #[test]
    fn button_routes_match_golden() {
        let got = serialize(&button_board());
        maybe_write_golden("src/wiring/goldens/button.routes.txt", &got);
        assert_eq!(
            got, BUTTON_GOLDEN,
            "button route golden drifted; if the change is intended, update crates/of-ui/src/wiring/goldens/button.routes.txt\n--- got ---\n{got}"
        );
    }

    #[test]
    fn button_invariants_hold() {
        let board = button_board();
        let got = check(&board).iter().map(|v| v.line()).collect::<Vec<_>>().join("\n");
        let got = if got.is_empty() { got } else { format!("{got}\n") };
        maybe_write_golden("src/wiring/goldens/button.invariants.txt", &got);
        assert_invariants_hold(
            "button",
            &board,
            BUTTON_INVARIANTS,
            "crates/of-ui/src/wiring/goldens/button.invariants.txt",
        );
    }

    #[test]
    fn button_router_is_deterministic() {
        assert_eq!(serialize(&button_board()), serialize(&button_board()));
    }
}
