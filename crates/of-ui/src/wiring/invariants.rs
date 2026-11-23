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
    out.extend(crossing_hops(schematic));
    out.extend(source_return_paths(schematic));
    out.extend(label_clearance(schematic));
    out.sort_by(|a, b| a.code.cmp(&b.code).then(a.detail.cmp(&b.detail)));
    out
}

fn fmt_pt(p: Point) -> String {
    format!("({}, {})", p.x, p.y)
}

fn body_crossings(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    for stroke in geometry_strokes(schematic) {
        for window in stroke.points.windows(2) {
            let a = window[0];
            let b = window[1];
            for comp in &schematic.components {
                if segment_hits_body(a, b, comp) {
                    out.push(Violation::new(
                        "body-crossing",
                        format!(
                            "{} {} segment {}–{} enters body {} (+ clearance {})",
                            stroke.kind,
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

/// One polyline used by the invariant checks: a hand-authored fixture stroke,
/// or a generated route polyline when the board has no fixtures.
struct GeometryStroke {
    kind: &'static str,
    id: String,
    nets: Vec<String>,
    points: Vec<Point>,
}

fn geometry_strokes(schematic: &Schematic) -> Vec<GeometryStroke> {
    if !schematic.fixture_strokes.is_empty() {
        return schematic
            .fixture_strokes
            .iter()
            .map(|s| GeometryStroke {
                kind: "stroke",
                id: s.id.clone(),
                nets: s.nets.clone(),
                points: s.points.clone(),
            })
            .collect();
    }
    let mut out = Vec::new();
    for net in &schematic.nets {
        let Some(route) = &net.route else {
            continue;
        };
        for (i, poly) in route.polylines.iter().enumerate() {
            out.push(GeometryStroke {
                kind: "route",
                id: format!("{}/{}", net.id, i),
                nets: vec![net.id.clone()],
                points: poly.clone(),
            });
        }
    }
    out
}

/// A segment may start or end at a pin of `comp`, but must not travel through
/// the body interior (inflated by clearance, except the uninflated body is
/// used at a pin so a stub can leave the outline).
pub(crate) fn segment_hits_body(a: Point, b: Point, comp: &Component) -> bool {
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

/// True if segment `a`–`b` enters the interior of `r`, or an endpoint sits inside it.
pub(crate) fn hits_rect(a: Point, b: Point, r: Rect) -> bool {
    segment_enters_interior(a, b, r) || point_in_interior(a, r) || point_in_interior(b, r)
}

fn point_on_or_in(p: Point, r: Rect) -> bool {
    p.x + EPS >= r.min.x && p.x <= r.max.x + EPS && p.y + EPS >= r.min.y && p.y <= r.max.y + EPS
}

pub(crate) fn point_in_interior(p: Point, r: Rect) -> bool {
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
    let strokes = geometry_strokes(schematic);
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
                            format!("{}s {} and {} overlap {}–{}", a.kind, a.id, b.id, fmt_pt(p), fmt_pt(q)),
                        ));
                    }
                }
            }
        }
    }
    out
}

/// Inclusive overlap of two colinear segments, if the shared length is > EPS.
pub(crate) fn colinear_overlap(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<(Point, Point)> {
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
        if deg >= 4 && !is_foreign_crossing_point(schematic, p) {
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
    for stroke in geometry_strokes(schematic) {
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
    for stroke in geometry_strokes(schematic) {
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

fn is_foreign_crossing_point(schematic: &Schematic, p: Point) -> bool {
    perpendicular_crossings(schematic)
        .iter()
        .any(|c| c.point.distance(p) <= EPS)
}

struct ForeignCrossing {
    net_a: String,
    net_b: String,
    point: Point,
    later: String,
    later_horizontal: bool,
}

fn kind_rank(kind: crate::wiring::model::NetKind) -> u8 {
    match kind {
        crate::wiring::model::NetKind::Power => 0,
        crate::wiring::model::NetKind::Ground => 1,
        crate::wiring::model::NetKind::Signal => 2,
    }
}

fn later_net(schematic: &Schematic, a: &str, b: &str) -> String {
    let na = schematic.net(a);
    let nb = schematic.net(b);
    let rank = |n: Option<&crate::wiring::model::Net>, id: &str| {
        n.map(|net| {
            (
                kind_rank(net.kind),
                std::cmp::Reverse(net.terminals.len()),
                id.to_string(),
            )
        })
        .unwrap_or((3, std::cmp::Reverse(0), id.to_string()))
    };
    if rank(na, a) <= rank(nb, b) {
        b.to_string()
    } else {
        a.to_string()
    }
}

fn segment_horizontal(a: Point, b: Point) -> Option<bool> {
    if (a.y - b.y).abs() <= EPS {
        Some(true)
    } else if (a.x - b.x).abs() <= EPS {
        Some(false)
    } else {
        None
    }
}

fn ortho_intersection(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point> {
    let a_h = (a0.y - a1.y).abs() <= EPS;
    let a_v = (a0.x - a1.x).abs() <= EPS;
    let b_h = (b0.y - b1.y).abs() <= EPS;
    let b_v = (b0.x - b1.x).abs() <= EPS;
    if a_h && b_v {
        Some(Point::new(b0.x, a0.y))
    } else if a_v && b_h {
        Some(Point::new(a0.x, b0.y))
    } else {
        None
    }
}

fn net_segments(schematic: &Schematic) -> Vec<(String, Point, Point)> {
    let mut out = Vec::new();
    for net in &schematic.nets {
        let Some(route) = &net.route else { continue };
        for poly in &route.polylines {
            for w in poly.windows(2) {
                out.push((net.id.clone(), w[0], w[1]));
            }
        }
    }
    out
}

fn perpendicular_crossings(schematic: &Schematic) -> Vec<ForeignCrossing> {
    let segs = net_segments(schematic);
    let mut out = Vec::new();
    for i in 0..segs.len() {
        for j in (i + 1)..segs.len() {
            let (ia, a0, a1) = &segs[i];
            let (ib, b0, b1) = &segs[j];
            if ia == ib {
                continue;
            }
            let Some(p) = ortho_intersection(*a0, *a1, *b0, *b1) else {
                continue;
            };
            if !crate::wiring::hops::on_open_segment(p, *a0, *a1) || !crate::wiring::hops::on_open_segment(p, *b0, *b1)
            {
                continue;
            }
            let later = later_net(schematic, ia, ib);
            let later_horizontal = if later == *ia {
                segment_horizontal(*a0, *a1).unwrap_or(false)
            } else {
                segment_horizontal(*b0, *b1).unwrap_or(false)
            };
            if out.iter().any(|c: &ForeignCrossing| {
                c.point.distance(p) <= EPS && {
                    let mut ids = [c.net_a.as_str(), c.net_b.as_str()];
                    ids.sort();
                    let mut other = [ia.as_str(), ib.as_str()];
                    other.sort();
                    ids == other
                }
            }) {
                continue;
            }
            out.push(ForeignCrossing {
                net_a: ia.clone(),
                net_b: ib.clone(),
                point: p,
                later,
                later_horizontal,
            });
        }
    }
    out
}

fn crossing_hops(schematic: &Schematic) -> Vec<Violation> {
    let mut out = Vec::new();
    let crossings = perpendicular_crossings(schematic);
    let mut hops: Vec<(String, crate::wiring::model::Hop)> = Vec::new();
    for net in &schematic.nets {
        if let Some(route) = &net.route {
            for hop in &route.hops {
                hops.push((net.id.clone(), *hop));
            }
        }
    }
    let declared = schematic.junctions();
    for c in &crossings {
        let here: Vec<&(String, crate::wiring::model::Hop)> =
            hops.iter().filter(|(_, h)| h.over.distance(c.point) <= EPS).collect();
        if here.is_empty() {
            out.push(Violation::new(
                "crossing-needs-hop",
                format!(
                    "foreign nets {} and {} cross at {} with no hop",
                    c.net_a,
                    c.net_b,
                    fmt_pt(c.point)
                ),
            ));
            continue;
        }
        if here.iter().any(|(n, _)| *n != c.later) {
            out.push(Violation::new(
                "hop-on-wrong-net",
                format!(
                    "crossing at {} should hop on later net {}, not {}",
                    fmt_pt(c.point),
                    c.later,
                    here.iter().map(|(n, _)| n.as_str()).collect::<Vec<_>>().join(",")
                ),
            ));
        }
        for (_, h) in &here {
            if h.across_horizontal != !c.later_horizontal {
                out.push(Violation::new(
                    "hop-orientation",
                    format!(
                        "hop at {} on {} has across_horizontal={}, expected {}",
                        fmt_pt(c.point),
                        c.later,
                        h.across_horizontal,
                        !c.later_horizontal
                    ),
                ));
            }
        }
    }
    for (net, hop) in &hops {
        let at_crossing = crossings.iter().any(|c| c.point.distance(hop.over) <= EPS);
        if !at_crossing {
            out.push(Violation::new(
                "hop-without-crossing",
                format!(
                    "hop on {net} at {} is not at a perpendicular foreign crossing",
                    fmt_pt(hop.over)
                ),
            ));
        }
        if declared.iter().any(|(_, q)| q.distance(hop.over) <= EPS) {
            out.push(Violation::new(
                "hop-at-junction",
                format!("hop on {net} at {} sits on a junction dot", fmt_pt(hop.over)),
            ));
        }
    }
    out
}

/// A bench PSU is a two-terminal source: + must be routed onto a power net
/// and - onto a ground net, so the drawn return is the same net the solver uses.
fn source_return_paths(schematic: &Schematic) -> Vec<Violation> {
    use crate::wiring::model::NetKind;
    let mut out = Vec::new();
    for comp in &schematic.components {
        let is_psu = comp.id == "psu" || comp.reference.eq_ignore_ascii_case("PSU");
        if !is_psu {
            continue;
        }
        let plus = comp.pins.iter().find(|p| p.id == "plus" || p.name == "+");
        let minus = comp
            .pins
            .iter()
            .find(|p| p.id == "minus" || p.name == "-" || p.name == "\u{2212}");
        match plus {
            Some(pin) => check_source_pin(schematic, &mut out, &comp.id, pin, NetKind::Power, "power"),
            None => out.push(Violation::new("source-open", format!("{} has no + pin", comp.id))),
        }
        match minus {
            Some(pin) => check_source_pin(schematic, &mut out, &comp.id, pin, NetKind::Ground, "ground"),
            None => out.push(Violation::new("source-open", format!("{} has no - pin", comp.id))),
        }
    }
    out
}

fn check_source_pin(
    schematic: &Schematic,
    out: &mut Vec<Violation>,
    component: &str,
    pin: &crate::wiring::model::Pin,
    expected: crate::wiring::model::NetKind,
    kind_name: &str,
) {
    if !pin.routed {
        out.push(Violation::new(
            "source-open",
            format!(
                "{}.{} is a name-only stub; it must be routed onto the {kind_name} net",
                component, pin.id
            ),
        ));
    }
    let Some(net_id) = pin.net.as_deref() else {
        out.push(Violation::new(
            "source-open",
            format!("{}.{} is not assigned to a net", component, pin.id),
        ));
        return;
    };
    let Some(net) = schematic.net(net_id) else {
        out.push(Violation::new(
            "source-open",
            format!("{}.{} names unknown net {net_id}", component, pin.id),
        ));
        return;
    };
    if net.kind != expected {
        out.push(Violation::new(
            "source-open",
            format!(
                "{}.{} is on {net_id} ({}) but must be on a {kind_name} net",
                component,
                pin.id,
                net.kind.as_str()
            ),
        ));
    }
    let on_route = net.route.as_ref().is_some_and(|route| {
        route
            .polylines
            .iter()
            .any(|poly| poly.iter().any(|p| p.distance(pin.position) <= EPS))
    });
    if pin.routed && !on_route {
        out.push(Violation::new(
            "source-open",
            format!("{}.{} is not joined into the routed {net_id} net", component, pin.id),
        ));
    }
}

/// Text labels (one-row keepouts) must not straddle or crowd a component's edge:
/// a label outside a body keeps a small gap from it. A label centred inside a
/// body is that part's own text (e.g. the SW1 caption) and is left alone.
fn label_clearance(schematic: &Schematic) -> Vec<Violation> {
    /// Taller keepouts are routing blocks, not text rows.
    const LABEL_ROW_MAX: f32 = 16.0;
    /// Minimum gap between a label box and a body edge.
    const LABEL_GAP: f32 = 2.0;
    let mut out = Vec::new();
    for k in &schematic.keepouts {
        if k.max.y - k.min.y > LABEL_ROW_MAX {
            continue;
        }
        for comp in &schematic.components {
            let b = comp.body;
            let centre = Point::new((k.min.x + k.max.x) * 0.5, (k.min.y + k.max.y) * 0.5);
            if point_in_interior(centre, b) {
                continue;
            }
            let overlaps = k.min.x - LABEL_GAP < b.max.x - EPS
                && k.max.x + LABEL_GAP > b.min.x + EPS
                && k.min.y - LABEL_GAP < b.max.y - EPS
                && k.max.y + LABEL_GAP > b.min.y + EPS;
            if overlaps {
                out.push(Violation::new(
                    "label-on-body",
                    format!(
                        "label box {}-{} is within {LABEL_GAP} px of {} body",
                        fmt_pt(k.min),
                        fmt_pt(k.max),
                        comp.id
                    ),
                ));
            }
        }
    }
    out
}
