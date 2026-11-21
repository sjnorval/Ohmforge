//! Geometry invariants the router (and today's fixtures) must satisfy.
//!
//! A check never rewrites the schematic: violations are reported so a hand-
//! authored fixture can be documented instead of being silently "fixed".

use crate::wiring::model::{CLEARANCE, Component, Direction, GRID, Point, Rect, Schematic};

const EPS: f32 = 1e-4;

/// One invariant failure, with a stable `code` used in documented snapshots.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Violation {
    pub code: String,
    pub detail: String,
}

impl Violation {
    fn new(code: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            detail: detail.into(),
        }
    }

    pub fn line(&self) -> String {
        format!("{}: {}", self.code, self.detail)
    }
}

/// Check the fixture routes in `schematic` against the standing conventions.
pub fn check(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    out.extend(body_crossings(schematic));
    out.extend(foreign_net_overlap(schematic));
    out.extend(junction_degrees(schematic));
    out.extend(pin_stubs(schematic));
    out.sort_by(|a, b| a.code.cmp(&b.code).then(a.detail.cmp(&b.detail)));
    out
}

fn fmt_pt(p: Point) -> String {
    format!("({}, {})", p.x, p.y)
}

fn body_crossings(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    for stroke in &schematic.fixture_strokes {
        for window in stroke.points.windows(2) {
            let a = window[0];
            let b = window[1];
            for comp in &schematic.components {
                if segment_hits_body(a, b, comp) {
                    out.push(Violation::new(
                        "body-crossing",
                        format!(
                            "stroke {} segment {}–{} enters body {} (+ clearance {})",
                            stroke.id,
                            fmt_pt(a),
                            fmt_pt(b),
                            comp.id,
                            CLEARANCE
                        ),
                    ));
                }
            }
        }
    }
    out
}

/// A segment may start or end at a pin of `comp`, but must not travel through
/// the body interior (inflated by clearance, except the uninflated body is
/// used at a pin so a stub can leave the outline).
fn segment_hits_body(a: Point, b: Point, comp: &Component) -> bool {
    let a_pin = pin_at(comp, a);
    let b_pin = pin_at(comp, b);
    // A stub or arrival at a pin of this component may sit on the outline;
    // only the uninflated interior is forbidden (so clearance is not applied
    // to the first grid step leaving the pin). Other segments must clear the
    // inflated body.
    let obstacle = if a_pin || b_pin {
        comp.body
    } else {
        comp.body.inflate(CLEARANCE)
    };
    let a_test = if a_pin { snap_to_boundary(a, comp.body) } else { a };
    let b_test = if b_pin { snap_to_boundary(b, comp.body) } else { b };
    segment_enters_interior(a_test, b_test, obstacle)
        || (!a_pin && point_in_interior(a, obstacle))
        || (!b_pin && point_in_interior(b, obstacle))
}

fn pin_at(comp: &Component, p: Point) -> bool {
    comp.pins.iter().any(|pin| pin.position.distance(p) <= EPS)
}

fn snap_to_boundary(p: Point, r: Rect) -> Point {
    // If p is inside or on the rect, project to the nearest edge so the stub's
    // first point is treated as being on the outline.
    if !point_in_interior(p, r) && !point_on_or_in(p, r) {
        return p;
    }
    let left = (p.x - r.min.x).abs();
    let right = (r.max.x - p.x).abs();
    let top = (p.y - r.min.y).abs();
    let bottom = (r.max.y - p.y).abs();
    let m = left.min(right).min(top).min(bottom);
    if m == left {
        Point::new(r.min.x, p.y)
    } else if m == right {
        Point::new(r.max.x, p.y)
    } else if m == top {
        Point::new(p.x, r.min.y)
    } else {
        Point::new(p.x, r.max.y)
    }
}

fn point_on_or_in(p: Point, r: Rect) -> bool {
    p.x + EPS >= r.min.x && p.x <= r.max.x + EPS && p.y + EPS >= r.min.y && p.y <= r.max.y + EPS
}

fn point_in_interior(p: Point, r: Rect) -> bool {
    p.x > r.min.x + EPS && p.x < r.max.x - EPS && p.y > r.min.y + EPS && p.y < r.max.y - EPS
}

fn segment_enters_interior(a: Point, b: Point, r: Rect) -> bool {
    // Liang-Barsky against a slightly inset rectangle (open interior).
    let inset = Rect {
        min: Point::new(r.min.x + EPS, r.min.y + EPS),
        max: Point::new(r.max.x - EPS, r.max.y - EPS),
    };
    if inset.max.x <= inset.min.x || inset.max.y <= inset.min.y {
        return false;
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let mut t0 = 0.0;
    let mut t1 = 1.0;
    if !clip(-dx, a.x - inset.min.x, &mut t0, &mut t1) {
        return false;
    }
    if !clip(dx, inset.max.x - a.x, &mut t0, &mut t1) {
        return false;
    }
    if !clip(-dy, a.y - inset.min.y, &mut t0, &mut t1) {
        return false;
    }
    if !clip(dy, inset.max.y - a.y, &mut t0, &mut t1) {
        return false;
    }
    t0 < t1 || (t0 - t1).abs() <= EPS && point_in_interior(Point::new(a.x + dx * t0, a.y + dy * t0), r)
}

fn clip(p: f32, q: f32, t0: &mut f32, t1: &mut f32) -> bool {
    if p.abs() <= EPS {
        return q >= -EPS;
    }
    let r = q / p;
    if p < 0.0 {
        if r > *t1 {
            return false;
        }
        if r > *t0 {
            *t0 = r;
        }
    } else {
        if r < *t0 {
            return false;
        }
        if r < *t1 {
            *t1 = r;
        }
    }
    true
}

fn foreign_net_overlap(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    let strokes = &schematic.fixture_strokes;
    for i in 0..strokes.len() {
        for j in (i + 1)..strokes.len() {
            let a = &strokes[i];
            let b = &strokes[j];
            let share = a.nets.iter().any(|n| b.nets.contains(n));
            if share {
                continue;
            }
            for wa in a.points.windows(2) {
                for wb in b.points.windows(2) {
                    if let Some((p, q)) = colinear_overlap(wa[0], wa[1], wb[0], wb[1]) {
                        out.push(Violation::new(
                            "foreign-overlap",
                            format!("strokes {} and {} overlap {}–{}", a.id, b.id, fmt_pt(p), fmt_pt(q)),
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Inclusive overlap of two colinear segments, if the shared length is > EPS.
fn colinear_overlap(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<(Point, Point)> {
    let ax = (a1.x - a0.x).abs() <= EPS;
    let ay = (a1.y - a0.y).abs() <= EPS;
    let bx = (b1.x - b0.x).abs() <= EPS;
    let by = (b1.y - b0.y).abs() <= EPS;
    if ax && bx && (a0.x - b0.x).abs() <= EPS {
        // both vertical, same x
        let (a_lo, a_hi) = minmax(a0.y, a1.y);
        let (b_lo, b_hi) = minmax(b0.y, b1.y);
        let lo = a_lo.max(b_lo);
        let hi = a_hi.min(b_hi);
        if hi - lo > EPS {
            return Some((Point::new(a0.x, lo), Point::new(a0.x, hi)));
        }
        return None;
    }
    if ay && by && (a0.y - b0.y).abs() <= EPS {
        let (a_lo, a_hi) = minmax(a0.x, a1.x);
        let (b_lo, b_hi) = minmax(b0.x, b1.x);
        let lo = a_lo.max(b_lo);
        let hi = a_hi.min(b_hi);
        if hi - lo > EPS {
            return Some((Point::new(lo, a0.y), Point::new(hi, a0.y)));
        }
        return None;
    }
    None
}

fn minmax(a: f32, b: f32) -> (f32, f32) {
    if a <= b { (a, b) } else { (b, a) }
}

fn junction_degrees(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    let segments = all_segments(schematic);
    let declared = schematic.junctions();
    for (net, p) in &declared {
        let deg = degree_at(&segments, *p);
        if deg != 3 {
            out.push(Violation::new(
                "junction-degree",
                format!("declared junction {net} at {} has degree {deg}, expected 3", fmt_pt(*p)),
            ));
        }
    }
    // Undeclared T-junctions (degree 3) or 4-way meetings.
    let mut candidates: Vec<Point> = Vec::new();
    for (a, b) in &segments {
        push_unique(&mut candidates, *a);
        push_unique(&mut candidates, *b);
        // Interior T: other segment endpoints that lie on this segment.
        for (c, d) in &segments {
            for p in [*c, *d] {
                if point_on_open_segment(p, *a, *b) {
                    push_unique(&mut candidates, p);
                }
            }
        }
    }
    for p in candidates {
        let deg = degree_at(&segments, p);
        let is_declared = declared.iter().any(|(_, q)| q.distance(p) <= EPS);
        if deg == 3 && !is_declared {
            out.push(Violation::new(
                "junction-missing",
                format!("degree-3 meeting at {} has no junction dot", fmt_pt(p)),
            ));
        }
        if deg >= 4 {
            out.push(Violation::new(
                "junction-four-way",
                format!("meeting at {} has degree {deg}", fmt_pt(p)),
            ));
        }
    }
    out
}

fn all_segments(schematic: &Schematic) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    for stroke in &schematic.fixture_strokes {
        for w in stroke.points.windows(2) {
            out.push((w[0], w[1]));
        }
    }
    out
}

fn degree_at(segments: &[(Point, Point)], p: Point) -> usize {
    let mut arms = 0;
    for &(a, b) in segments {
        if a.distance(p) <= EPS || b.distance(p) <= EPS {
            arms += 1;
        } else if point_on_open_segment(p, a, b) {
            arms += 2;
        }
    }
    arms
}

fn point_on_open_segment(p: Point, a: Point, b: Point) -> bool {
    let ab = a.distance(b);
    if ab <= EPS {
        return false;
    }
    let ap = a.distance(p);
    let pb = p.distance(b);
    (ap + pb - ab).abs() <= EPS && ap > EPS && pb > EPS
}

fn push_unique(points: &mut Vec<Point>, p: Point) {
    if !points.iter().any(|q| q.distance(p) <= EPS) {
        points.push(p);
    }
}

fn pin_stubs(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    for comp in &schematic.components {
        for pin in &comp.pins {
            let Some(net_id) = pin.net.as_deref() else {
                continue;
            };
            match stub_from_pin(schematic, pin.position, pin.direction, net_id) {
                Stub::Ok(len) => {
                    if len + EPS < GRID {
                        out.push(Violation::new(
                            "stub-short",
                            format!("pin {}.{} stub length {len} < grid {GRID}", comp.id, pin.id),
                        ));
                    }
                }
                Stub::WrongWay => {
                    out.push(Violation::new(
                        "stub-direction",
                        format!(
                            "pin {}.{} does not leave {} from {}",
                            comp.id,
                            pin.id,
                            pin.direction.as_str(),
                            fmt_pt(pin.position)
                        ),
                    ));
                }
                Stub::Missing => {
                    out.push(Violation::new(
                        "stub-missing",
                        format!(
                            "pin {}.{} has no route vertex at {}",
                            comp.id,
                            pin.id,
                            fmt_pt(pin.position)
                        ),
                    ));
                }
            }
        }
    }
    out
}

enum Stub {
    Ok(f32),
    WrongWay,
    Missing,
}

fn stub_from_pin(schematic: &Schematic, pos: Point, dir: Direction, net_id: &str) -> Stub {
    let (vx, vy) = dir.vector();
    let mut best: Option<f32> = None;
    let mut saw = false;
    for stroke in &schematic.fixture_strokes {
        if !stroke.nets.iter().any(|n| n == net_id) {
            continue;
        }
        let pts = &stroke.points;
        for (i, p) in pts.iter().enumerate() {
            if p.distance(pos) > EPS {
                continue;
            }
            saw = true;
            let mut neighbors = Vec::new();
            if i > 0 {
                neighbors.push(pts[i - 1]);
            }
            if i + 1 < pts.len() {
                neighbors.push(pts[i + 1]);
            }
            for n in neighbors {
                let dx = n.x - pos.x;
                let dy = n.y - pos.y;
                let len = pos.distance(n);
                if len <= EPS {
                    continue;
                }
                let ux = dx / len;
                let uy = dy / len;
                if (ux - vx).abs() <= 0.05 && (uy - vy).abs() <= 0.05 {
                    best = Some(best.map_or(len, |b| b.max(len)));
                }
            }
        }
    }
    if let Some(len) = best {
        Stub::Ok(len)
    } else if saw {
        Stub::WrongWay
    } else {
        Stub::Missing
    }
}
