//! Hop geometry: a small semicircle on the later-routed wire at a perpendicular
//! foreign-net crossing.
//!
//! The hopping wire is the later net in the deterministic route order (power,
//! ground, then fanout, then name). A vertical hopping wire bulges east; a
//! horizontal hopping wire bulges north (decreasing y). Junction dots are never
//! placed at a crossing.

use crate::wiring::model::{GRID, Hop, Point};

/// Radius of the hop semicircle, in schematic units (one grid step).
pub const HOP_RADIUS: f32 = GRID;

const EPS: f32 = 1e-4;
const ARC_STEPS: usize = 8;

/// Whether `p` lies on the open segment `a`–`b` (not at an endpoint).
pub fn on_open_segment(p: Point, a: Point, b: Point) -> bool {
    if p.distance(a) <= EPS || p.distance(b) <= EPS {
        return false;
    }
    let ab = a.distance(b);
    if ab <= EPS {
        return false;
    }
    (a.distance(p) + p.distance(b) - ab).abs() <= 1e-3
}

/// Split a polyline at hops on its segments, returning straight runs and hop arcs
/// in travel order, ready to stroke.
pub fn poly_with_hops(pts: &[Point], hops: &[Hop]) -> Vec<Vec<Point>> {
    if pts.len() < 2 {
        return Vec::new();
    }
    let mut pieces: Vec<Vec<Point>> = Vec::new();
    let mut current: Vec<Point> = vec![pts[0]];
    for w in pts.windows(2) {
        let a = w[0];
        let b = w[1];
        let mut on_seg: Vec<&Hop> = hops.iter().filter(|h| on_open_segment(h.over, a, b)).collect();
        on_seg.sort_by(|x, y| {
            a.distance(x.over)
                .total_cmp(&a.distance(y.over))
                .then(x.over.x.total_cmp(&y.over.x))
                .then(x.over.y.total_cmp(&y.over.y))
        });
        for hop in on_seg {
            let radius = hop_radius(hop.over, a, b);
            if radius <= EPS {
                continue;
            }
            let (approach, depart) = approach_depart(hop.over, a, b, radius);
            if current.last().is_none_or(|p| p.distance(approach) > EPS) {
                current.push(approach);
            }
            if current.len() >= 2 {
                pieces.push(std::mem::take(&mut current));
            } else {
                current.clear();
            }
            let arc = hop_arc(*hop, approach, depart, radius);
            if arc.len() >= 2 {
                pieces.push(arc);
            }
            current = vec![depart];
        }
        if current.last().is_none_or(|p| p.distance(b) > EPS) {
            current.push(b);
        }
    }
    if current.len() >= 2 {
        pieces.push(current);
    }
    pieces
}

fn hop_radius(over: Point, a: Point, b: Point) -> f32 {
    let room = a.distance(over).min(b.distance(over));
    if room <= EPS {
        return 0.0;
    }
    HOP_RADIUS.min(room * 0.8)
}

fn approach_depart(over: Point, a: Point, b: Point, radius: f32) -> (Point, Point) {
    let to_a = Point::new(a.x - over.x, a.y - over.y);
    let to_b = Point::new(b.x - over.x, b.y - over.y);
    let la = over.distance(a);
    let lb = over.distance(b);
    let approach = Point::new(over.x + to_a.x / la * radius, over.y + to_a.y / la * radius);
    let depart = Point::new(over.x + to_b.x / lb * radius, over.y + to_b.y / lb * radius);
    (approach, depart)
}

/// Semicircular arc from `from` to `to` around `hop.over`.
pub fn hop_arc(hop: Hop, from: Point, to: Point, radius: f32) -> Vec<Point> {
    let c = hop.over;
    if hop.across_horizontal {
        // Vertical hopping wire, bulge east.
        let start = if from.y >= to.y { 0.0 } else { std::f32::consts::PI };
        let end = if from.y >= to.y { std::f32::consts::PI } else { 0.0 };
        sample_arc(c, radius, start, end, |theta, r| {
            Point::new(c.x + r * theta.sin(), c.y + r * theta.cos())
        })
    } else {
        // Horizontal hopping wire, bulge north (decreasing y).
        let start = if from.x <= to.x { 0.0 } else { std::f32::consts::PI };
        let end = if from.x <= to.x { std::f32::consts::PI } else { 0.0 };
        sample_arc(c, radius, start, end, |phi, r| {
            Point::new(c.x - r * phi.cos(), c.y - r * phi.sin())
        })
    }
}

fn sample_arc(_c: Point, radius: f32, start: f32, end: f32, point: impl Fn(f32, f32) -> Point) -> Vec<Point> {
    let mut pts = Vec::with_capacity(ARC_STEPS + 1);
    for i in 0..=ARC_STEPS {
        let t = i as f32 / ARC_STEPS as f32;
        let theta = start + (end - start) * t;
        pts.push(point(theta, radius));
    }
    pts
}
