//! Geometry, components, nets and routes for a schematic.
//!
//! Types in this module are plain data: they do not depend on egui. The GUI
//! converts [`Point`] and [`Rect`] at paint time so the same model can be
//! tested headlessly.

/// Provisional routing grid, in schematic units.
///
/// The first router PR may pick 4 or 8; invariants and stub lengths use this
/// starting value (see `docs/design/wire-routing.md`, open question 1).
pub const GRID: f32 = 4.0;

/// Starting clearance between a route and a foreign body: one grid step.
pub const CLEARANCE: f32 = GRID;

/// A point in schematic space (not screen pixels).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Point {
    pub x: f32,
    pub y: f32,
}

impl Point {
    pub const fn new(x: f32, y: f32) -> Self {
        Self { x, y }
    }

    pub fn distance(self, other: Self) -> f32 {
        let dx = self.x - other.x;
        let dy = self.y - other.y;
        dx.hypot(dy)
    }
}

/// Axis-aligned rectangle in schematic space, `min` inclusive, `max` inclusive.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Point,
    pub max: Point,
}

impl Rect {
    pub fn from_min_max(min: Point, max: Point) -> Self {
        Self { min, max }
    }

    pub fn inflate(self, amount: f32) -> Self {
        Self {
            min: Point::new(self.min.x - amount, self.min.y - amount),
            max: Point::new(self.max.x + amount, self.max.y + amount),
        }
    }
}

/// Departure direction of a pin stub: away from the symbol, along the grid.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    North,
    East,
    South,
    West,
}

impl Direction {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::North => "N",
            Self::East => "E",
            Self::South => "S",
            Self::West => "W",
        }
    }

    pub fn vector(self) -> (f32, f32) {
        match self {
            Self::North => (0.0, -1.0),
            Self::East => (1.0, 0.0),
            Self::South => (0.0, 1.0),
            Self::West => (-1.0, 0.0),
        }
    }
}

/// Kind of net, used for routing order and default stroke.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum NetKind {
    Signal,
    Power,
    Ground,
}

impl NetKind {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Signal => "signal",
            Self::Power => "power",
            Self::Ground => "ground",
        }
    }
}

/// How a hand-authored fixture stroke is painted.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StrokeStyle {
    /// Muted 1.7 px wire (ground return, stubs).
    Wire,
    /// 2.0 px teal when the GPIO/LED net is on, otherwise muted 1.7 px.
    LiveSignal,
    /// Always 1.7 px teal (VDD rail).
    Power,
}

impl StrokeStyle {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Wire => "wire",
            Self::LiveSignal => "live",
            Self::Power => "power",
        }
    }
}

/// A pin on a component, in schematic space.
#[derive(Clone, Debug, PartialEq)]
pub struct Pin {
    pub id: String,
    pub name: String,
    pub position: Point,
    pub direction: Direction,
    pub net: Option<String>,
    /// When `false`, the router emits a perpendicular stub only and does not
    /// join the pin to the net trunk. Used for labelled power or ground pins
    /// that connect by name (see `docs/design/wire-routing.md` §2 and open
    /// question 3).
    pub routed: bool,
}

/// A placed symbol. `body` is the routing obstacle, not necessarily the hover hit target.
#[derive(Clone, Debug, PartialEq)]
pub struct Component {
    pub id: String,
    pub reference: String,
    pub body: Rect,
    pub pins: Vec<Pin>,
}

/// A pin terminal on a net: (`component` id, `pin` id).
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PinRef {
    pub component: String,
    pub pin: String,
}

impl PinRef {
    pub fn new(component: impl Into<String>, pin: impl Into<String>) -> Self {
        Self {
            component: component.into(),
            pin: pin.into(),
        }
    }
}

/// A hop/bridge where two foreign nets cross at a point.
///
/// The later-routed net hops. `across_horizontal` is true when the crossed
/// wire is horizontal, so the hop is a vertical semicircle bulging east.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Hop {
    pub over: Point,
    /// The crossed wire is horizontal, so the hop is a vertical semicircle.
    pub across_horizontal: bool,
}

/// Routed geometry for a net: polylines, degree-3 junctions, and optional hops.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Route {
    pub polylines: Vec<Vec<Point>>,
    pub junctions: Vec<Point>,
    pub hops: Vec<Hop>,
}

/// A named net with terminals, optional waypoints (empty until the editor) and a cached route.
#[derive(Clone, Debug, PartialEq)]
pub struct Net {
    pub id: String,
    pub name: String,
    pub kind: NetKind,
    pub terminals: Vec<PinRef>,
    pub waypoints: Vec<Point>,
    pub route: Option<Route>,
}

/// One hand-authored stroke, in the order the demo paints net geometry.
///
/// A stroke may belong to more than one net when a historical polyline runs
/// through a series symbol (the GPIO–R1–anode line on the blink board).
#[derive(Clone, Debug, PartialEq)]
pub struct FixtureStroke {
    pub id: String,
    pub style: StrokeStyle,
    /// Net ids this stroke belongs to.
    pub nets: Vec<String>,
    pub points: Vec<Point>,
    /// The blink PSU-minus tick is painted with `line_segment`, not `Shape::line`.
    pub line_segment: bool,
}

/// A complete schematic: components, nets, and (for PR 1) fixture strokes.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Schematic {
    pub components: Vec<Component>,
    pub nets: Vec<Net>,
    pub fixture_strokes: Vec<FixtureStroke>,
    /// Label and annotation boxes the router must not run through. Clearance
    /// is already included in the rect; these are not component bodies.
    pub keepouts: Vec<Rect>,
}

impl Schematic {
    pub fn component(&self, id: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.id == id)
    }

    pub fn net(&self, id: &str) -> Option<&Net> {
        self.nets.iter().find(|n| n.id == id)
    }

    pub fn stroke(&self, id: &str) -> Option<&FixtureStroke> {
        self.fixture_strokes.iter().find(|s| s.id == id)
    }

    pub fn junctions(&self) -> Vec<(String, Point)> {
        let mut out = Vec::new();
        for net in &self.nets {
            if let Some(route) = &net.route {
                for &p in &route.junctions {
                    out.push((net.id.clone(), p));
                }
            }
        }
        out.sort_by(|a, b| {
            a.1.y
                .total_cmp(&b.1.y)
                .then(a.1.x.total_cmp(&b.1.x))
                .then(a.0.cmp(&b.0))
        });
        out
    }
}
