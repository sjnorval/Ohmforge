//! Symbol geometry the painters draw that is not a routed net.

use super::model::Point;

/// The series-resistor zigzag, in schematic units, from R1 pin 1 to pin 2.
///
/// This is part of the R1 symbol and is painted with the live signal stroke so
/// the zig-zag still lights with the GPIO/LED net, but it is not a routed net
/// segment and must not be treated as a body crossing.
pub fn r1_zigzag() -> Vec<Point> {
    [
        (157.0, 82.0),
        (162.0, 74.0),
        (170.0, 90.0),
        (178.0, 74.0),
        (186.0, 90.0),
        (194.0, 74.0),
        (202.0, 90.0),
        (207.0, 82.0),
    ]
    .into_iter()
    .map(|(x, y)| Point::new(x, y))
    .collect()
}
