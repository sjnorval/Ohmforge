//! Deterministic orthogonal maze router for schematic nets.
//!
//! Routes are generated on a sparse grid: a [`GRID`] pitch plus tracks through
//! every pin, stub landing, body edge and keepout. Multi-source A* (Lee's
//! algorithm once a trunk exists) connects terminals with a unit step cost, a
//! heavy bend penalty, and a surcharge on tracks that stab a symbol so nets
//! prefer the streets between bodies. Existing same-net trunk cells are
//! sources at cost zero (trunk reuse). Foreign-net colinear overlap is
//! blocked. Perpendicular crossings are allowed at a high cost; the later-
//! routed net receives a hop glyph at each crossing.
//!
//! Nets are routed in a fixed order: power, then ground, then remaining nets
//! by descending fanout and a stable name order. The same parts, pins, nets
//! and waypoints therefore always produce the same geometry.
//!
//! The search is in-tree (no `pathfinding` crate) so the workspace stays on
//! the existing `deny.toml` allow list.

use std::cmp::Ordering;
use std::collections::{BTreeSet, BinaryHeap, HashMap, HashSet};

use crate::wiring::invariants::{colinear_overlap, hits_rect, segment_hits_body};
use crate::wiring::model::{
    CLEARANCE, Component, Direction, GRID, Hop, Net, NetKind, Pin, PinRef, Point, Rect, Route, Schematic,
};

/// Hundredths of a schematic unit, so pin coordinates that are already on a
/// whole pixel stay exact and the 4 px grid is an integer stride.
const Q: i32 = 100;
const GRID_Q: i32 = (GRID * Q as f32) as i32;
/// Padding around the component bounding box, in schematic units, so a net can
/// run a rail past the leftmost or rightmost symbol (the blink VDD rail sits
/// 10 px left of U1).
const PAD: f32 = 96.0;
/// A bend costs the same as 8 grid steps of travel, which favours long
/// straight runs without forcing a large detour to avoid a single corner.
const BEND_PENALTY: i64 = 8 * GRID_Q as i64;
/// Tracks whose infinite line stabs a body or keepout pay this multiplier so
/// the router prefers the streets between symbols (for example the GND return
/// under the chip rather than a tight run through the LED labels).
const OFF_STREET: i64 = 2;
/// A perpendicular foreign-net crossing costs 20 grid steps, so a short
/// path that must cross a rail is preferred to looping around the board,
/// while blink-style routes that never need a crossing stay unchanged.
const CROSS_PENALTY: i64 = 20 * GRID_Q as i64;

fn q(v: f32) -> i32 {
    (v * Q as f32).round() as i32
}

fn unq(v: i32) -> f32 {
    v as f32 / Q as f32
}

fn pq(p: Point) -> Cell {
    Cell { x: q(p.x), y: q(p.y) }
}

fn unp(c: Cell) -> Point {
    Point::new(unq(c.x), unq(c.y))
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash)]
struct Cell {
    x: i32,
    y: i32,
}

impl Cell {
    fn manhattan(self, other: Cell) -> i64 {
        (self.x - other.x).abs() as i64 + (self.y - other.y).abs() as i64
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Hash, PartialOrd, Ord)]
enum Heading {
    None,
    East,
    South,
    West,
    North,
}

impl Heading {
    fn between(a: Cell, b: Cell) -> Self {
        match (b.x.cmp(&a.x), b.y.cmp(&a.y)) {
            (Ordering::Greater, _) => Self::East,
            (Ordering::Less, _) => Self::West,
            (_, Ordering::Greater) => Self::South,
            (_, Ordering::Less) => Self::North,
            _ => Self::None,
        }
    }
}

/// Open-set entry. [`BinaryHeap`] is a max-heap, so [`Ord`] is reversed on `f`.
#[derive(Clone, Copy, Eq, PartialEq)]
struct Open {
    f: i64,
    x: i32,
    y: i32,
    heading: Heading,
}

impl Ord for Open {
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .f
            .cmp(&self.f)
            .then(self.x.cmp(&other.x))
            .then(self.y.cmp(&other.y))
            .then(self.heading.cmp(&other.heading))
    }
}

impl PartialOrd for Open {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

struct Grid {
    xs: Vec<i32>,
    ys: Vec<i32>,
}

impl Grid {
    fn build(schematic: &Schematic) -> Self {
        let mut min_x = i32::MAX;
        let mut max_x = i32::MIN;
        let mut min_y = i32::MAX;
        let mut max_y = i32::MIN;
        let mut xs = BTreeSet::new();
        let mut ys = BTreeSet::new();

        let mut consider = |x: i32, y: i32| {
            min_x = min_x.min(x);
            max_x = max_x.max(x);
            min_y = min_y.min(y);
            max_y = max_y.max(y);
            xs.insert(x);
            ys.insert(y);
        };

        for comp in &schematic.components {
            consider(q(comp.body.min.x), q(comp.body.min.y));
            consider(q(comp.body.max.x), q(comp.body.max.y));
            let inflated = comp.body.inflate(CLEARANCE);
            consider(q(inflated.min.x), q(inflated.min.y));
            consider(q(inflated.max.x), q(inflated.max.y));
            for pin in &comp.pins {
                consider(q(pin.position.x), q(pin.position.y));
                let end = stub_end(pin, schematic);
                consider(q(end.x), q(end.y));
            }
        }
        for keepout in &schematic.keepouts {
            consider(q(keepout.min.x), q(keepout.min.y));
            consider(q(keepout.max.x), q(keepout.max.y));
        }

        if min_x == i32::MAX {
            return Self {
                xs: Vec::new(),
                ys: Vec::new(),
            };
        }

        let pad = q(PAD);
        min_x -= pad;
        max_x += pad;
        min_y -= pad;
        max_y += pad;

        let mut x = align_down(min_x, GRID_Q);
        while x <= max_x {
            xs.insert(x);
            x += GRID_Q;
        }
        let mut y = align_down(min_y, GRID_Q);
        while y <= max_y {
            ys.insert(y);
            y += GRID_Q;
        }

        Self {
            xs: xs.into_iter().collect(),
            ys: ys.into_iter().collect(),
        }
    }

    fn neighbors(&self, cell: Cell) -> Vec<Cell> {
        let mut out = Vec::with_capacity(4);
        if let Ok(i) = self.xs.binary_search(&cell.x) {
            if i > 0 {
                out.push(Cell {
                    x: self.xs[i - 1],
                    y: cell.y,
                });
            }
            if i + 1 < self.xs.len() {
                out.push(Cell {
                    x: self.xs[i + 1],
                    y: cell.y,
                });
            }
        }
        if let Ok(i) = self.ys.binary_search(&cell.y) {
            if i > 0 {
                out.push(Cell {
                    x: cell.x,
                    y: self.ys[i - 1],
                });
            }
            if i + 1 < self.ys.len() {
                out.push(Cell {
                    x: cell.x,
                    y: self.ys[i + 1],
                });
            }
        }
        out
    }

    fn on_grid(&self, cell: Cell) -> bool {
        self.xs.binary_search(&cell.x).is_ok() && self.ys.binary_search(&cell.y).is_ok()
    }

    /// Every grid point that lies on the segment `a`–`b`, inclusive.
    fn cells_on(&self, a: Point, b: Point) -> Vec<Cell> {
        let a = pq(a);
        let b = pq(b);
        let mut out = Vec::new();
        if a.x == b.x {
            let x = a.x;
            let (lo, hi) = if a.y <= b.y { (a.y, b.y) } else { (b.y, a.y) };
            for &y in &self.ys {
                if y >= lo && y <= hi {
                    out.push(Cell { x, y });
                }
            }
        } else if a.y == b.y {
            let y = a.y;
            let (lo, hi) = if a.x <= b.x { (a.x, b.x) } else { (b.x, a.x) };
            for &x in &self.xs {
                if x >= lo && x <= hi {
                    out.push(Cell { x, y });
                }
            }
        } else {
            out.push(a);
            out.push(b);
        }
        out
    }
}

fn align_down(v: i32, step: i32) -> i32 {
    let r = v.rem_euclid(step);
    v - r
}

/// Generate orthogonal routes for every net in `schematic`.
///
/// Existing [`crate::wiring::model::Net::route`] caches are replaced. Pins
/// with [`Pin::routed`] set to `false` receive a perpendicular stub only
/// (labelled power or ground) and are not joined to the net trunk. After a
/// successful routing, hops are assigned on the later-routed net at every
/// perpendicular foreign-net crossing.
pub fn route(schematic: &mut Schematic) {
    for net in &mut schematic.nets {
        net.route = None;
    }
    let order = net_order(schematic);
    let mut placed: Vec<Placed> = Vec::new();
    for id in &order {
        if let Some(generated) = route_net(schematic, id, &placed) {
            placed.push(Placed {
                id: id.clone(),
                segments: polyline_segments(&generated.polylines),
            });
            if let Some(net) = schematic.nets.iter_mut().find(|n| n.id == *id) {
                net.route = Some(generated);
            }
        }
    }
    assign_hops(schematic, &order);
}

struct Crossing {
    net_a: String,
    net_b: String,
    point: Point,
    a_horizontal: bool,
    b_horizontal: bool,
}

fn interior_crossings(schematic: &Schematic) -> Vec<Crossing> {
    let mut out = Vec::new();
    let mut nets: Vec<(&str, Vec<(Point, Point)>)> = Vec::new();
    for net in &schematic.nets {
        let Some(route) = &net.route else { continue };
        nets.push((net.id.as_str(), polyline_segments(&route.polylines)));
    }
    for i in 0..nets.len() {
        for j in (i + 1)..nets.len() {
            let (id_a, segs_a) = &nets[i];
            let (id_b, segs_b) = &nets[j];
            for &(a0, a1) in segs_a {
                for &(b0, b1) in segs_b {
                    if let Some(p) = interior_cross_point(a0, a1, b0, b1) {
                        out.push(Crossing {
                            net_a: (*id_a).to_string(),
                            net_b: (*id_b).to_string(),
                            point: p,
                            a_horizontal: (a0.y - a1.y).abs() <= 1e-4,
                            b_horizontal: (b0.y - b1.y).abs() <= 1e-4,
                        });
                    }
                }
            }
        }
    }
    out.sort_by(|a, b| {
        a.point
            .y
            .total_cmp(&b.point.y)
            .then(a.point.x.total_cmp(&b.point.x))
            .then(a.net_a.cmp(&b.net_a))
            .then(a.net_b.cmp(&b.net_b))
    });
    out
}

fn interior_cross_point(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point> {
    let p = ortho_intersection(a0, a1, b0, b1)?;
    if crate::wiring::hops::on_open_segment(p, a0, a1) && crate::wiring::hops::on_open_segment(p, b0, b1) {
        Some(p)
    } else {
        None
    }
}

fn assign_hops(schematic: &mut Schematic, order: &[String]) {
    let rank = |id: &str| order.iter().position(|x| x == id).unwrap_or(usize::MAX);
    let mut hops_by_net: HashMap<String, Vec<Hop>> = HashMap::new();
    for c in interior_crossings(schematic) {
        let later = if rank(&c.net_a) > rank(&c.net_b) {
            c.net_a.clone()
        } else if rank(&c.net_b) > rank(&c.net_a) {
            c.net_b.clone()
        } else if c.net_a <= c.net_b {
            c.net_a.clone()
        } else {
            c.net_b.clone()
        };
        let later_horizontal = if later == c.net_a {
            c.a_horizontal
        } else {
            c.b_horizontal
        };
        hops_by_net.entry(later).or_default().push(Hop {
            over: c.point,
            across_horizontal: !later_horizontal,
        });
    }
    for net in &mut schematic.nets {
        let Some(route) = &mut net.route else { continue };
        let mut hops = hops_by_net.remove(&net.id).unwrap_or_default();
        hops.sort_by(|a, b| a.over.y.total_cmp(&b.over.y).then(a.over.x.total_cmp(&b.over.x)));
        route.hops = hops;
    }
}

struct Placed {
    #[allow(dead_code)]
    id: String,
    segments: Vec<(Point, Point)>,
}

struct Terminal {
    pin_ref: PinRef,
    position: Point,
    #[allow(dead_code)]
    direction: Direction,
    routed: bool,
}

pub(crate) fn net_order(schematic: &Schematic) -> Vec<String> {
    let mut nets: Vec<&Net> = schematic.nets.iter().collect();
    nets.sort_by(|a, b| {
        kind_rank(a.kind)
            .cmp(&kind_rank(b.kind))
            .then(b.terminals.len().cmp(&a.terminals.len()))
            .then(a.id.cmp(&b.id))
    });
    nets.into_iter().map(|n| n.id.clone()).collect()
}

fn kind_rank(kind: NetKind) -> u8 {
    match kind {
        NetKind::Power => 0,
        NetKind::Ground => 1,
        NetKind::Signal => 2,
    }
}

fn terminals_for(schematic: &Schematic, net: &Net) -> Vec<Terminal> {
    let mut out = Vec::new();
    for pin_ref in &net.terminals {
        if let Some((pin, _)) = find_pin(schematic, pin_ref) {
            out.push(Terminal {
                pin_ref: pin_ref.clone(),
                position: pin.position,
                direction: pin.direction,
                routed: pin.routed,
            });
        }
    }
    out
}

fn find_pin<'a>(schematic: &'a Schematic, pin_ref: &PinRef) -> Option<(&'a Pin, &'a Component)> {
    let comp = schematic.component(&pin_ref.component)?;
    let pin = comp.pins.iter().find(|p| p.id == pin_ref.pin)?;
    Some((pin, comp))
}

fn route_net(schematic: &Schematic, net_id: &str, placed: &[Placed]) -> Option<Route> {
    let net = schematic.net(net_id)?;
    let terms = terminals_for(schematic, net);
    if terms.is_empty() {
        return Some(Route::default());
    }

    let grid = Grid::build(schematic);
    let foreign: Vec<(Point, Point)> = placed.iter().flat_map(|p| p.segments.iter().copied()).collect();
    let mut trunk: Vec<(Point, Point)> = Vec::new();

    let stubs: Vec<(Point, Point)> = terms
        .iter()
        .map(|t| {
            let pin = find_pin(schematic, &t.pin_ref).map(|(p, _)| p);
            let end = pin.map(|p| stub_end(p, schematic)).unwrap_or(t.position);
            (t.position, end)
        })
        .collect();

    let routed: Vec<usize> = terms
        .iter()
        .enumerate()
        .filter(|(_, t)| t.routed)
        .map(|(i, _)| i)
        .collect();

    if routed.len() == 1 {
        trunk.push(stubs[routed[0]]);
    } else if routed.len() >= 2 {
        let (i, j) = farthest_pair(&terms, &routed);
        trunk.push(stubs[i]);
        let goal = stubs[j].1;
        let path = astar(&grid, schematic, net_id, &trunk, stubs[i].1, goal, &foreign)?;
        append_path(&mut trunk, &path);
        trunk.push(stubs[j]);

        let mut remaining: Vec<usize> = routed.into_iter().filter(|&k| k != i && k != j).collect();
        remaining.sort_by(|&a, &b| term_id(&terms[a]).cmp(&term_id(&terms[b])));
        while !remaining.is_empty() {
            let (idx_in_remaining, term_idx) = closest_to_trunk(&terms, &stubs, &remaining, &trunk);
            remaining.remove(idx_in_remaining);
            let goal = stubs[term_idx].1;
            // Already on the trunk (the stub landing sits on an existing segment).
            if point_on_trunk(goal, &trunk) {
                trunk.push(stubs[term_idx]);
                continue;
            }
            let path = astar(
                &grid,
                schematic,
                net_id,
                &trunk,
                /* unused start, multi-source from trunk */ stubs[term_idx].1,
                goal,
                &foreign,
            )?;
            append_path(&mut trunk, &path);
            trunk.push(stubs[term_idx]);
        }
    }

    // Labelled power/ground pins emit a perpendicular stub only, after the
    // trunk is complete, so they cannot become A* sources for other pins.
    for (term, stub) in terms.iter().zip(&stubs) {
        if !term.routed {
            trunk.push(*stub);
        }
    }

    Some(finish_route(&trunk))
}

fn term_id(term: &Terminal) -> (String, String) {
    (term.pin_ref.component.clone(), term.pin_ref.pin.clone())
}

fn farthest_pair(terms: &[Terminal], routed: &[usize]) -> (usize, usize) {
    let mut best = (routed[0], routed[1]);
    let mut best_d = -1.0f32;
    for (a_i, &a) in routed.iter().enumerate() {
        for &b in &routed[a_i + 1..] {
            let d = terms[a].position.distance(terms[b].position);
            let key_a = term_id(&terms[a]);
            let key_b = term_id(&terms[b]);
            let pair = if key_a <= key_b { (a, b) } else { (b, a) };
            let better = d > best_d + 1e-4
                || ((d - best_d).abs() <= 1e-4
                    && (term_id(&terms[pair.0]), term_id(&terms[pair.1]))
                        < (term_id(&terms[best.0]), term_id(&terms[best.1])));
            if better {
                best_d = d;
                best = pair;
            }
        }
    }
    best
}

fn closest_to_trunk(
    terms: &[Terminal],
    stubs: &[(Point, Point)],
    remaining: &[usize],
    trunk: &[(Point, Point)],
) -> (usize, usize) {
    let mut best_i = 0;
    let mut best_d = f32::MAX;
    let mut best_id = term_id(&terms[remaining[0]]);
    for (i, &t) in remaining.iter().enumerate() {
        let goal = stubs[t].1;
        let d = min_distance_to_trunk(goal, trunk);
        let id = term_id(&terms[t]);
        if d < best_d - 1e-4 || ((d - best_d).abs() <= 1e-4 && id < best_id) {
            best_d = d;
            best_i = i;
            best_id = id;
        }
    }
    (best_i, remaining[best_i])
}

fn min_distance_to_trunk(p: Point, trunk: &[(Point, Point)]) -> f32 {
    let mut best = f32::MAX;
    for &(a, b) in trunk {
        best = best.min(distance_to_segment(p, a, b));
    }
    best
}

fn distance_to_segment(p: Point, a: Point, b: Point) -> f32 {
    let ab = a.distance(b);
    if ab <= 1e-4 {
        return p.distance(a);
    }
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    let t = ((p.x - a.x) * dx + (p.y - a.y) * dy) / (ab * ab);
    let t = t.clamp(0.0, 1.0);
    p.distance(Point::new(a.x + dx * t, a.y + dy * t))
}

fn point_on_trunk(p: Point, trunk: &[(Point, Point)]) -> bool {
    trunk.iter().any(|&(a, b)| {
        let ab = a.distance(b);
        if ab <= 1e-4 {
            return p.distance(a) <= 1e-4;
        }
        let ap = a.distance(p);
        let pb = p.distance(b);
        (ap + pb - ab).abs() <= 1e-3
    })
}

fn append_path(trunk: &mut Vec<(Point, Point)>, path: &[Point]) {
    for w in path.windows(2) {
        if w[0].distance(w[1]) > 1e-4 {
            trunk.push((w[0], w[1]));
        }
    }
}

/// First grid point at least one [`GRID`] step from the pin in its departure
/// direction that sits on or outside the inflated body (and any keepout).
pub(crate) fn stub_end(pin: &Pin, schematic: &Schematic) -> Point {
    let (vx, vy) = pin.direction.vector();
    let mut len = GRID;
    for _ in 0..64 {
        let end = Point::new(pin.position.x + vx * len, pin.position.y + vy * len);
        let mut clear = true;
        for comp in &schematic.components {
            let obstacle = comp.body.inflate(CLEARANCE);
            if point_in_interior_pub(end, obstacle) {
                clear = false;
                break;
            }
        }
        if clear {
            for keepout in &schematic.keepouts {
                if point_in_interior_pub(end, *keepout) {
                    clear = false;
                    break;
                }
            }
        }
        if clear {
            return end;
        }
        len += GRID;
    }
    Point::new(pin.position.x + vx * len, pin.position.y + vy * len)
}

fn point_in_interior_pub(p: Point, r: Rect) -> bool {
    crate::wiring::invariants::point_in_interior(p, r)
}

fn astar(
    grid: &Grid,
    schematic: &Schematic,
    net_id: &str,
    trunk: &[(Point, Point)],
    hint_start: Point,
    goal: Point,
    foreign: &[(Point, Point)],
) -> Option<Vec<Point>> {
    let goal_cell = pq(goal);
    if !grid.on_grid(goal_cell) {
        return None;
    }

    let mut sources: HashSet<Cell> = HashSet::new();
    for &(a, b) in trunk {
        for c in grid.cells_on(a, b) {
            sources.insert(c);
        }
    }
    if sources.is_empty() {
        sources.insert(pq(hint_start));
    }

    let mut open = BinaryHeap::new();
    let mut g_score: HashMap<Cell, i64> = HashMap::new();
    let mut came: HashMap<Cell, Cell> = HashMap::new();
    let mut came_head: HashMap<Cell, Heading> = HashMap::new();

    let mut starts: Vec<Cell> = sources.into_iter().collect();
    starts.sort_by_key(|c| (c.x, c.y));
    for s in starts {
        g_score.insert(s, 0);
        came_head.insert(s, Heading::None);
        open.push(Open {
            f: s.manhattan(goal_cell),
            x: s.x,
            y: s.y,
            heading: Heading::None,
        });
    }

    let mut visited: HashSet<(i32, i32, Heading)> = HashSet::new();

    while let Some(cur) = open.pop() {
        let cell = Cell { x: cur.x, y: cur.y };
        if !visited.insert((cur.x, cur.y, cur.heading)) {
            continue;
        }
        let Some(&g_cur) = g_score.get(&cell) else {
            continue;
        };
        // Stale heap entry (a better g was found later).
        if cur.f < g_cur + cell.manhattan(goal_cell) && cur.heading != *came_head.get(&cell).unwrap_or(&Heading::None) {
            // still explore; heading-specific
        }
        if cell == goal_cell {
            return Some(reconstruct(&came, cell, &g_score));
        }
        let heading = *came_head.get(&cell).unwrap_or(&Heading::None);
        for neigh in grid.neighbors(cell) {
            let edge_a = unp(cell);
            let edge_b = unp(neigh);
            if edge_blocked(schematic, net_id, edge_a, edge_b, trunk, foreign) {
                continue;
            }
            let dir = Heading::between(cell, neigh);
            let dist = cell.manhattan(neigh);
            let mul = if street_edge(schematic, edge_a, edge_b) {
                1
            } else {
                OFF_STREET
            };
            let mut ng = g_cur + dist * mul;
            if heading != Heading::None && heading != dir {
                ng += BEND_PENALTY;
            }
            if foreign_crossing(edge_a, edge_b, foreign) {
                ng += CROSS_PENALTY;
            }
            if ng < *g_score.get(&neigh).unwrap_or(&i64::MAX) {
                g_score.insert(neigh, ng);
                came.insert(neigh, cell);
                came_head.insert(neigh, dir);
                open.push(Open {
                    f: ng + neigh.manhattan(goal_cell),
                    x: neigh.x,
                    y: neigh.y,
                    heading: dir,
                });
            }
        }
    }
    None
}

fn reconstruct(came: &HashMap<Cell, Cell>, mut cell: Cell, _g: &HashMap<Cell, i64>) -> Vec<Point> {
    let mut cells = vec![cell];
    let mut guard = 0;
    while let Some(&prev) = came.get(&cell) {
        if prev == cell {
            break;
        }
        cells.push(prev);
        cell = prev;
        guard += 1;
        if guard > 100_000 {
            break;
        }
    }
    cells.reverse();
    let points: Vec<Point> = cells.into_iter().map(unp).collect();
    simplify(&points)
}

fn simplify(points: &[Point]) -> Vec<Point> {
    if points.len() <= 2 {
        return points.to_vec();
    }
    let mut out = vec![points[0]];
    for &p in &points[1..] {
        if out.last().is_some_and(|q| q.distance(p) <= 1e-4) {
            continue;
        }
        if out.len() >= 2 {
            let a = out[out.len() - 2];
            let b = out[out.len() - 1];
            if colinear_same_axis(a, b, p) {
                out.pop();
            }
        }
        out.push(p);
    }
    out
}

fn colinear_same_axis(a: Point, b: Point, c: Point) -> bool {
    let horiz = (a.y - b.y).abs() <= 1e-4 && (b.y - c.y).abs() <= 1e-4;
    let vert = (a.x - b.x).abs() <= 1e-4 && (b.x - c.x).abs() <= 1e-4;
    horiz || vert
}

fn hits_rect_closed(a: Point, b: Point, r: crate::wiring::model::Rect) -> bool {
    // A stub landing may sit on the inflated outline. The next step may leave
    // that outline, but must not travel along it or enter the interior.
    let on_or_in =
        |p: Point| p.x + 1e-4 >= r.min.x && p.x <= r.max.x + 1e-4 && p.y + 1e-4 >= r.min.y && p.y <= r.max.y + 1e-4;
    if crate::wiring::invariants::point_in_interior(a, r) || crate::wiring::invariants::point_in_interior(b, r) {
        return true;
    }
    if hits_rect(a, b, r) {
        return true;
    }
    // Both endpoints on the boundary means an axis-aligned walk along an edge.
    if on_or_in(a) && on_or_in(b) {
        return true;
    }
    false
}

fn edge_blocked(
    schematic: &Schematic,
    _net_id: &str,
    a: Point,
    b: Point,
    _trunk: &[(Point, Point)],
    foreign: &[(Point, Point)],
) -> bool {
    for comp in &schematic.components {
        // Pin stubs may sit on the uninflated outline (same rule as the
        // invariant). Other segments must stay strictly outside the inflated
        // body, including the clearance boundary itself.
        let a_pin = comp.pins.iter().any(|pin| pin.position.distance(a) <= 1e-4);
        let b_pin = comp.pins.iter().any(|pin| pin.position.distance(b) <= 1e-4);
        if a_pin || b_pin {
            if segment_hits_body(a, b, comp) {
                return true;
            }
        } else if hits_rect_closed(a, b, comp.body.inflate(CLEARANCE)) {
            return true;
        }
    }
    for keepout in &schematic.keepouts {
        if hits_rect(a, b, *keepout) {
            return true;
        }
    }
    for &(fa, fb) in foreign {
        if colinear_overlap(a, b, fa, fb).is_some() {
            return true;
        }
    }
    false
}

fn foreign_crossing(a: Point, b: Point, foreign: &[(Point, Point)]) -> bool {
    foreign.iter().any(|&(fa, fb)| proper_cross_or_touch(a, b, fa, fb))
}

fn proper_cross_or_touch(a0: Point, a1: Point, b0: Point, b1: Point) -> bool {
    if let Some(p) = ortho_intersection(a0, a1, b0, b1) {
        let on_a = on_segment_inclusive(p, a0, a1);
        let on_b = on_segment_inclusive(p, b0, b1);
        return on_a && on_b;
    }
    false
}

fn ortho_intersection(a0: Point, a1: Point, b0: Point, b1: Point) -> Option<Point> {
    let a_h = (a0.y - a1.y).abs() <= 1e-4;
    let a_v = (a0.x - a1.x).abs() <= 1e-4;
    let b_h = (b0.y - b1.y).abs() <= 1e-4;
    let b_v = (b0.x - b1.x).abs() <= 1e-4;
    if a_h && b_v {
        return Some(Point::new(b0.x, a0.y));
    }
    if a_v && b_h {
        return Some(Point::new(a0.x, b0.y));
    }
    None
}

fn on_segment_inclusive(p: Point, a: Point, b: Point) -> bool {
    let (lo_x, hi_x) = if a.x <= b.x { (a.x, b.x) } else { (b.x, a.x) };
    let (lo_y, hi_y) = if a.y <= b.y { (a.y, b.y) } else { (b.y, a.y) };
    p.x + 1e-4 >= lo_x && p.x <= hi_x + 1e-4 && p.y + 1e-4 >= lo_y && p.y <= hi_y + 1e-4
}

fn street_edge(schematic: &Schematic, a: Point, b: Point) -> bool {
    let horiz = (a.y - b.y).abs() <= 1e-4;
    let coord = if horiz { a.y } else { a.x };
    !track_stabs(schematic, horiz, coord)
}

fn track_stabs(schematic: &Schematic, horizontal: bool, coord: f32) -> bool {
    for comp in &schematic.components {
        let r = comp.body.inflate(CLEARANCE);
        if horizontal {
            if coord + 1e-4 >= r.min.y && coord <= r.max.y + 1e-4 {
                return true;
            }
        } else if coord + 1e-4 >= r.min.x && coord <= r.max.x + 1e-4 {
            return true;
        }
    }
    for keepout in &schematic.keepouts {
        if horizontal {
            if coord > keepout.min.y + 1e-4 && coord < keepout.max.y - 1e-4 {
                return true;
            }
        } else if coord > keepout.min.x + 1e-4 && coord < keepout.max.x - 1e-4 {
            return true;
        }
    }
    false
}

fn finish_route(trunk: &[(Point, Point)]) -> Route {
    let segments: Vec<(Point, Point)> = trunk.iter().copied().filter(|(a, b)| a.distance(*b) > 1e-4).collect();
    let polylines = segments_to_polylines(&segments);
    let junctions = degree3_points(&segments);
    Route {
        polylines,
        junctions,
        hops: Vec::new(),
    }
}

fn polyline_segments(polylines: &[Vec<Point>]) -> Vec<(Point, Point)> {
    let mut out = Vec::new();
    for poly in polylines {
        for w in poly.windows(2) {
            out.push((w[0], w[1]));
        }
    }
    out
}

fn segments_to_polylines(segments: &[(Point, Point)]) -> Vec<Vec<Point>> {
    let mut adj: HashMap<(i32, i32), Vec<Point>> = HashMap::new();
    let key = |p: Point| (q(p.x), q(p.y));
    for &(a, b) in segments {
        adj.entry(key(a)).or_default().push(b);
        adj.entry(key(b)).or_default().push(a);
    }
    // Unique neighbours at each point (round-trip of a doubled segment).
    for neighbours in adj.values_mut() {
        let mut uniq: Vec<Point> = Vec::new();
        for p in neighbours.drain(..) {
            if !uniq.iter().any(|q| q.distance(p) <= 1e-4) {
                uniq.push(p);
            }
        }
        *neighbours = uniq;
    }

    let mut used: HashSet<((i32, i32), (i32, i32))> = HashSet::new();
    let edge_key = |a: Point, b: Point| {
        let ka = key(a);
        let kb = key(b);
        if ka <= kb { (ka, kb) } else { (kb, ka) }
    };

    let mut polylines = Vec::new();
    // Start walks at degree-1 endpoints first, then any leftover (loops).
    let mut starts: Vec<Point> = Vec::new();
    for &(a, b) in segments {
        for p in [a, b] {
            let deg = adj.get(&key(p)).map(Vec::len).unwrap_or(0);
            if deg <= 1 && !starts.iter().any(|q| q.distance(p) <= 1e-4) {
                starts.push(p);
            }
        }
    }
    for &(a, _) in segments {
        if !starts.iter().any(|q| q.distance(a) <= 1e-4) {
            starts.push(a);
        }
    }

    for start in starts {
        while let Some(neighs) = adj.get(&key(start)) {
            let next = neighs.iter().copied().find(|&n| used.insert(edge_key(start, n)));
            let Some(cur_next) = next else { break };
            let mut poly = vec![start, cur_next];
            let mut cur = cur_next;
            loop {
                let deg = adj.get(&key(cur)).map(Vec::len).unwrap_or(0);
                // Stop at a junction (degree != 2) so a T becomes two polylines
                // that share the junction vertex.
                if deg != 2 {
                    break;
                }
                let Some(neighs) = adj.get(&key(cur)) else { break };
                let Some(n) = neighs.iter().copied().find(|&n| used.insert(edge_key(cur, n))) else {
                    break;
                };
                poly.push(n);
                cur = n;
            }
            let poly = simplify(&poly);
            if poly.len() >= 2 {
                polylines.push(poly);
            }
        }
    }

    // Stable paint order: left-to-right, then top-to-bottom, by first vertex.
    polylines.sort_by(|a, b| {
        a[0].y
            .total_cmp(&b[0].y)
            .then(a[0].x.total_cmp(&b[0].x))
            .then(a[a.len() - 1].y.total_cmp(&b[b.len() - 1].y))
            .then(a[a.len() - 1].x.total_cmp(&b[b.len() - 1].x))
    });
    polylines
}

fn degree3_points(segments: &[(Point, Point)]) -> Vec<Point> {
    let mut points: Vec<Point> = Vec::new();
    let push = |pts: &mut Vec<Point>, p: Point| {
        if !pts.iter().any(|q| q.distance(p) <= 1e-4) {
            pts.push(p);
        }
    };
    for &(a, b) in segments {
        push(&mut points, a);
        push(&mut points, b);
        for &(c, d) in segments {
            for p in [c, d] {
                if point_on_open(p, a, b) {
                    push(&mut points, p);
                }
            }
        }
    }
    let mut junctions = Vec::new();
    for p in points {
        let deg = degree_at(segments, p);
        if deg == 3 {
            junctions.push(p);
        } else if deg >= 4 {
            // A later rip-up/jog pass will split 4-way meetings; record nothing
            // so the invariant check (not a silent fix) can still flag them.
        }
    }
    junctions.sort_by(|a, b| a.y.total_cmp(&b.y).then(a.x.total_cmp(&b.x)));
    junctions
}

fn point_on_open(p: Point, a: Point, b: Point) -> bool {
    let ab = a.distance(b);
    if ab <= 1e-4 {
        return false;
    }
    let ap = a.distance(p);
    let pb = p.distance(b);
    (ap + pb - ab).abs() <= 1e-3 && ap > 1e-4 && pb > 1e-4
}

fn degree_at(segments: &[(Point, Point)], p: Point) -> usize {
    let mut arms = 0;
    for &(a, b) in segments {
        if a.distance(p) <= 1e-4 || b.distance(p) <= 1e-4 {
            arms += 1;
        } else if point_on_open(p, a, b) {
            arms += 2;
        }
    }
    arms
}
