//! Deterministic text serialisation of a [`Schematic`].

use crate::wiring::model::{Hop, Point, Schematic};

fn fmt_coord(v: f32) -> String {
    if v.is_finite() && v == v.trunc() {
        format!("{}", v as i32)
    } else {
        format!("{v}")
    }
}

fn fmt_point(p: Point) -> String {
    format!("{},{}", fmt_coord(p.x), fmt_coord(p.y))
}

fn fmt_points(points: &[Point]) -> String {
    points.iter().map(|p| fmt_point(*p)).collect::<Vec<_>>().join(" ")
}

/// Serialise `schematic` to a stable, line-oriented text form.
///
/// Components and nets are sorted by id. Fixture strokes stay in paint order
/// (that order is part of the golden). Junctions and hops are sorted by
/// position then net id.
pub fn serialize(schematic: &Schematic) -> String {
    let mut out = String::from("# schematic routes\n");
    out.push_str(&format!(
        "# grid={} clearance={}\n",
        crate::wiring::GRID,
        crate::wiring::CLEARANCE
    ));

    let mut components = schematic.components.clone();
    components.sort_by(|a, b| a.id.cmp(&b.id));
    for c in &components {
        out.push_str(&format!(
            "component {} {} {} {}\n",
            c.id,
            c.reference.replace(' ', "_"),
            fmt_point(c.body.min),
            fmt_point(c.body.max)
        ));
        for pin in &c.pins {
            let net = pin.net.as_deref().unwrap_or("-");
            let stub = if pin.routed { "" } else { " stub" };
            out.push_str(&format!(
                "  pin {} {} {} {}{}\n",
                pin.id,
                fmt_point(pin.position),
                pin.direction.as_str(),
                net,
                stub
            ));
        }
    }

    let mut nets = schematic.nets.clone();
    nets.sort_by(|a, b| a.id.cmp(&b.id));
    for n in &nets {
        let terms = n
            .terminals
            .iter()
            .map(|t| format!("{}.{}", t.component, t.pin))
            .collect::<Vec<_>>()
            .join(" ");
        out.push_str(&format!("net {} {} {}\n", n.id, n.kind.as_str(), terms));
        if !n.waypoints.is_empty() {
            let pts = n.waypoints.iter().map(|p| fmt_point(*p)).collect::<Vec<_>>().join(" ");
            out.push_str(&format!("  waypoints {pts}\n"));
        }
        if let Some(route) = &n.route {
            for poly in &route.polylines {
                out.push_str(&format!("  route {}\n", fmt_points(poly)));
            }
        }
    }

    for keepout in &schematic.keepouts {
        out.push_str(&format!(
            "keepout {} {}\n",
            fmt_point(keepout.min),
            fmt_point(keepout.max)
        ));
    }

    for s in &schematic.fixture_strokes {
        let nets = s.nets.join(",");
        let seg = if s.line_segment { " segment" } else { "" };
        out.push_str(&format!("stroke {} {} {}{}\n", s.id, s.style.as_str(), nets, seg));
        out.push_str(&format!("  {}\n", fmt_points(&s.points)));
    }

    let mut junctions = schematic.junctions();
    junctions.sort_by(|a, b| {
        a.1.y
            .total_cmp(&b.1.y)
            .then(a.1.x.total_cmp(&b.1.x))
            .then(a.0.cmp(&b.0))
    });
    for (net, p) in junctions {
        out.push_str(&format!("junction {net} {}\n", fmt_point(p)));
    }

    let mut hops: Vec<(String, Hop)> = Vec::new();
    for net in &schematic.nets {
        if let Some(route) = &net.route {
            for hop in &route.hops {
                hops.push((net.id.clone(), *hop));
            }
        }
    }
    hops.sort_by(|a, b| {
        a.1.over
            .y
            .total_cmp(&b.1.over.y)
            .then(a.1.over.x.total_cmp(&b.1.over.x))
            .then(a.0.cmp(&b.0))
    });
    for (net, hop) in hops {
        let axis = if hop.across_horizontal { "h" } else { "v" };
        out.push_str(&format!("hop {net} {} {axis}\n", fmt_point(hop.over)));
    }

    out
}
