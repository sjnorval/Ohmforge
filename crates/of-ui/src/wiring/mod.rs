//! Schematic wiring model, fixture routes, and geometry invariants.
//!
//! This module is pure Rust and is compiled without the `gui` feature so the
//! goldens and invariants run in the workspace test job.

mod blink;
mod golden;
mod invariants;
mod model;

pub use model::{
    CLEARANCE, Component, Direction, FixtureStroke, GRID, Hop, Net, NetKind, Pin, PinRef, Point, Rect, Route,
    Schematic, StrokeStyle,
};

pub use blink::board as blink_board;
pub use golden::serialize;
pub use invariants::check;

#[cfg(test)]
mod tests {
    use super::{Hop, Point, StrokeStyle, blink_board, check, serialize};

    const BLINK_GOLDEN: &str = include_str!("goldens/blink.routes.txt");
    const BLINK_INVARIANTS: &str = include_str!("goldens/blink.invariants.txt");

    #[test]
    fn blink_serialise_is_deterministic() {
        let board = blink_board();
        assert!(board.component("u1").is_some());
        assert!(board.net("gnd").is_some());
        assert!(board.stroke("vdd").is_some());
        let a = serialize(&board);
        let b = serialize(&blink_board());
        assert_eq!(a, b);
    }

    #[test]
    fn blink_fixture_matches_golden() {
        let got = serialize(&blink_board());
        assert_eq!(
            got, BLINK_GOLDEN,
            "blink fixture golden drifted; if the change is intended, update crates/of-ui/src/wiring/goldens/blink.routes.txt\n--- got ---\n{got}"
        );
    }

    #[test]
    fn blink_invariants_are_documented() {
        // Today's hand-authored blink paths violate some router conventions
        // (the R1 zigzag is painted as part of the net, and the VDD/minus
        // strokes go through or into the PSU body). The snapshot documents
        // that; do not change the pixels to silence it.
        let got = check(&blink_board())
            .iter()
            .map(|v| v.line())
            .collect::<Vec<_>>()
            .join("\n");
        let got = if got.is_empty() { got } else { format!("{got}\n") };
        assert_eq!(
            got, BLINK_INVARIANTS,
            "blink invariant snapshot drifted; if today's hand-authored paths still look the same, update crates/of-ui/src/wiring/goldens/blink.invariants.txt\n--- got ---\n{got}"
        );
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
        };
        assert_eq!(check(&schematic), Vec::new(), "{:?}", check(&schematic));
    }
}
