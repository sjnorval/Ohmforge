//! Schematic wiring model, fixture routes, and geometry invariants.
//!
//! This module is pure Rust and is compiled without the `gui` feature so the
//! goldens and invariants run in the workspace test job.

mod blink;
mod button;
mod dimmer;
#[cfg(test)]
mod from_circuit;
mod golden;
mod hops;
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
pub use hops::poly_with_hops;
pub use invariants::check;
pub use router::route;

#[cfg(test)]
mod tests {
    use super::{
        Hop, Point, StrokeStyle, blink_board, button_board, check, dimmer_board, dimmer_board_at, fade_board,
        poly_with_hops, serialize,
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
        maybe_write_golden("src/wiring/goldens/blink.routes.txt", &got);
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
        maybe_write_golden("src/wiring/goldens/blink.invariants.txt", &got);
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
        // Hop fields are part of the golden serialisation and the paint helper.
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

    fn hops_on(board: &super::Schematic, net_id: &str) -> Vec<Point> {
        board
            .net(net_id)
            .and_then(|n| n.route.as_ref())
            .map(|r| r.hops.iter().map(|h| h.over).collect())
            .unwrap_or_default()
    }

    #[test]
    fn hop_at_every_crossing_and_none_at_junctions() {
        for (name, board) in [
            ("blink", blink_board()),
            ("fade", fade_board()),
            ("dimmer", dimmer_board()),
            ("button", button_board()),
        ] {
            let v = check(&board);
            assert!(
                v.is_empty(),
                "{name} invariants failed:\n{}",
                v.iter().map(|x| x.line()).collect::<Vec<_>>().join("\n")
            );
            let junctions: Vec<_> = board.junctions().into_iter().map(|(_, p)| p).collect();
            for net in &board.nets {
                let Some(route) = &net.route else { continue };
                for hop in &route.hops {
                    assert!(
                        !junctions.iter().any(|j| j.distance(hop.over) <= 1e-4),
                        "{name} hop on {} at ({}, {}) sits on a junction",
                        net.id,
                        hop.over.x,
                        hop.over.y
                    );
                }
            }
        }
        let dimmer = dimmer_board();
        let gpio2_hops = hops_on(&dimmer, "gpio2");
        assert_eq!(
            gpio2_hops.len(),
            2,
            "dimmer gpio2 should hop over VDD and GND, hops={gpio2_hops:?}"
        );
        assert!(hops_on(&dimmer, "vdd").is_empty());
        // GND return (including the PSU minus) hops over foreign nets (VDD and
        // the pot wiper rail) on the way to the common ground symbol.
        assert_eq!(
            hops_on(&dimmer, "gnd").len(),
            2,
            "dimmer gnd hops={:?}",
            hops_on(&dimmer, "gnd")
        );
        let button = button_board();
        let gpio9_hops = hops_on(&button, "gpio9");
        assert_eq!(
            gpio9_hops.len(),
            1,
            "button gpio9 should hop over GND, hops={gpio9_hops:?}"
        );
        assert!(hops_on(&button, "gnd").is_empty());
        assert!(hops_on(&blink_board(), "gnd").is_empty());
        assert!(hops_on(&fade_board(), "vdd").is_empty());
    }

    #[test]
    fn hop_arc_splits_a_vertical_wire() {
        let hop = Hop {
            over: Point::new(10.0, 20.0),
            across_horizontal: true,
        };
        let pieces = poly_with_hops(&[Point::new(10.0, 0.0), Point::new(10.0, 40.0)], &[hop]);
        assert!(
            pieces.len() >= 3,
            "expected approach, arc, depart, got {}",
            pieces.len()
        );
        let arc = &pieces[1];
        assert!(arc.len() >= 3);
        // Vertical hop bulges east of the crossing.
        assert!(arc.iter().any(|p| p.x > 10.0 + 1.0));
        assert!(arc.iter().all(|p| p.x >= 10.0 - 1e-3));
    }

    #[test]
    fn rip_up_resolves_a_synthetic_blocked_corridor() {
        use super::{Component, Direction, Net, NetKind, Pin, PinRef, Rect, Schematic};
        let pin = |id: &str, x: f32, y: f32, dir: Direction, net: &str| Pin {
            id: id.into(),
            name: id.into(),
            position: Point::new(x, y),
            direction: dir,
            net: Some(net.into()),
            routed: true,
        };
        let comp = |id: &str, min: (f32, f32), max: (f32, f32), pins: Vec<Pin>| Component {
            id: id.into(),
            reference: id.to_uppercase(),
            body: Rect::from_min_max(Point::new(min.0, min.1), Point::new(max.0, max.1)),
            pins,
        };
        let ko =
            |min: (f32, f32), max: (f32, f32)| Rect::from_min_max(Point::new(min.0, min.1), Point::new(max.0, max.1));
        let mut board = Schematic {
            components: vec![
                comp(
                    "ea",
                    (-16.0, 16.0),
                    (-4.0, 24.0),
                    vec![pin("p", -4.0, 20.0, Direction::East, "early")],
                ),
                comp(
                    "eb",
                    (132.0, 16.0),
                    (144.0, 24.0),
                    vec![pin("p", 132.0, 20.0, Direction::West, "early")],
                ),
                comp(
                    "la",
                    (16.0, 28.0),
                    (24.0, 36.0),
                    vec![pin("p", 20.0, 28.0, Direction::North, "late")],
                ),
                comp(
                    "lb",
                    (104.0, 28.0),
                    (112.0, 36.0),
                    vec![pin("p", 108.0, 28.0, Direction::North, "late")],
                ),
            ],
            nets: vec![
                Net {
                    id: "early".into(),
                    name: "early".into(),
                    kind: NetKind::Power,
                    terminals: vec![PinRef::new("ea", "p"), PinRef::new("eb", "p")],
                    waypoints: Vec::new(),
                    route: None,
                },
                Net {
                    id: "late".into(),
                    name: "late".into(),
                    kind: NetKind::Signal,
                    terminals: vec![PinRef::new("la", "p"), PinRef::new("lb", "p")],
                    waypoints: Vec::new(),
                    route: None,
                },
            ],
            fixture_strokes: Vec::new(),
            keepouts: vec![
                // Ceiling of the mid corridor; early can still go up at x=-4 / 132.
                ko((12.0, 8.0), (116.0, 18.0)),
                // Floor of the corridor, with gaps at the late verticals (x=20 and x=108).
                ko((24.0, 24.0), (100.0, 80.0)),
                ko((-20.0, 24.0), (16.0, 80.0)),
                ko((112.0, 24.0), (160.0, 80.0)),
            ],
        };
        super::route(&mut board);
        let violations = check(&board);
        assert!(
            violations.is_empty(),
            "synthetic board invariants:\n{}",
            violations.iter().map(|v| v.line()).collect::<Vec<_>>().join("\n")
        );
        let early = board.net("early").and_then(|n| n.route.as_ref()).expect("early routed");
        let late = board.net("late").and_then(|n| n.route.as_ref()).expect("late routed");
        assert!(!early.polylines.is_empty());
        assert!(!late.polylines.is_empty());
        let ys = |route: &super::Route| {
            route
                .polylines
                .iter()
                .flat_map(|p| p.iter())
                .map(|pt| pt.y)
                .collect::<Vec<_>>()
        };
        let late_ys = ys(late);
        let early_ys = ys(early);
        assert!(
            late_ys.iter().any(|y| (*y - 20.0).abs() <= 1e-3),
            "late should use the mid corridor, ys={late_ys:?}"
        );
        assert!(
            early_ys.iter().any(|y| *y <= 8.0),
            "early should detour over the top after rip-up, ys={early_ys:?}"
        );
    }

    #[test]
    fn every_demo_routes_the_psu_return() {
        let boards = [
            ("blink", blink_board()),
            ("dimmer", dimmer_board()),
            ("fade", fade_board()),
            ("button", button_board()),
        ];
        for (name, board) in boards {
            let psu = board.component("psu").unwrap_or_else(|| panic!("{name} has no psu"));
            let plus = psu.pins.iter().find(|p| p.id == "plus").expect("plus");
            let minus = psu.pins.iter().find(|p| p.id == "minus").expect("minus");
            assert!(plus.routed, "{name} psu.plus must be routed to VDD");
            assert!(minus.routed, "{name} psu.minus must be routed to GND");
            assert_eq!(plus.net.as_deref(), Some("vdd"), "{name} psu.plus net");
            assert_eq!(minus.net.as_deref(), Some("gnd"), "{name} psu.minus net");
            let gnd = board.net("gnd").and_then(|n| n.route.as_ref()).expect("gnd routed");
            let on_gnd = gnd
                .polylines
                .iter()
                .any(|poly| poly.iter().any(|p| p.distance(minus.position) <= 1e-4));
            assert!(on_gnd, "{name} psu.minus is not on the GND route");
            let vdd = board.net("vdd").and_then(|n| n.route.as_ref()).expect("vdd routed");
            let on_vdd = vdd
                .polylines
                .iter()
                .any(|poly| poly.iter().any(|p| p.distance(plus.position) <= 1e-4));
            assert!(on_vdd, "{name} psu.plus is not on the VDD route");
            assert!(check(&board).is_empty(), "{name} invariants: {:?}", check(&board));
        }
    }

    #[test]
    fn a_label_straddling_a_body_edge_is_flagged() {
        // The old "GND under PSU" box sat across the PSU's bottom-right corner.
        let mut board = blink_board();
        assert!(check(&board).is_empty(), "{:?}", check(&board));
        board.keepouts.push(crate::wiring::model::Rect::from_min_max(
            Point::new(85.0, 260.0),
            Point::new(107.0, 272.0),
        ));
        let v = check(&board);
        assert!(
            v.iter().any(|v| v.code == "label-on-body" && v.detail.contains("psu")),
            "expected a label-on-body violation for the PSU, got {v:?}"
        );
    }
}
