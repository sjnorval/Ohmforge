//! Schematic overlay for probe points and placed meters (existing stroke language).

use egui::{Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};

use of_instruments::Reading;

use crate::meters::{MeterKind, PlacedMeter, ProbeRef};
use crate::theme::TEAL;

/// A clickable node on the schematic, in schematic coordinates.
#[derive(Clone, Copy)]
pub struct ProbePoint {
    pub id: &'static str,
    pub node: &'static str,
    pub label: &'static str,
    pub xy: (f32, f32),
}

/// What the user is doing with meters this frame.
pub struct MeterView<'a> {
    pub arm: Option<MeterKind>,
    pub pending_a: Option<&'a str>,
    pub meters: &'a [PlacedMeter],
    /// The worker's readings, by meter id.
    pub readings: &'a [(u64, Reading)],
}

impl MeterView<'_> {
    /// The display text of meter `id` (`—` until the worker has read it).
    pub fn text(&self, id: u64) -> String {
        reading_text(self.readings, id)
    }
}

/// The display text of meter `id` in `readings`.
pub fn reading_text(readings: &[(u64, Reading)], id: u64) -> String {
    readings
        .iter()
        .find(|(i, _)| *i == id)
        .map_or(Reading::Missing, |(_, r)| r.clone())
        .display()
}

/// Result of interacting with the schematic overlay.
#[derive(Default)]
pub struct SchematicHit {
    pub button: bool,
    pub probe: Option<ProbeRef>,
}

/// Draw probe targets (when placing) and live meter labels; return a clicked probe.
pub fn meter_layer(
    ui: &Ui,
    point: impl Fn(f32, f32) -> Pos2,
    painter: &egui::Painter,
    probes: &[ProbePoint],
    view: &MeterView<'_>,
    muted: Color32,
) -> Option<ProbeRef> {
    let mut clicked = None;
    if view.arm.is_some() {
        for p in probes {
            let c = point(p.xy.0, p.xy.1);
            let rect = Rect::from_center_size(c, egui::vec2(14.0, 14.0));
            let id = ui.scope_id().with(("probe", p.id));
            let r = ui.interact(rect, id, Sense::click());
            let armed_a = view.pending_a == Some(p.id);
            let color = if r.hovered() || armed_a { TEAL } else { muted };
            painter.circle_stroke(c, 4.0, Stroke::new(1.7, color));
            if r.hovered() {
                r.clone().on_hover_text(p.label);
            }
            if r.clicked() {
                clicked = Some(ProbeRef {
                    id: p.id.to_owned(),
                    node: p.node.to_owned(),
                    label: p.label.to_owned(),
                });
            }
        }
    }

    for meter in view.meters {
        let Some(pa) = probes.iter().find(|p| p.id == meter.a.id) else {
            continue;
        };
        let Some(pb) = probes.iter().find(|p| p.id == meter.b.id) else {
            continue;
        };
        let a = point(pa.xy.0, pa.xy.1);
        let b = point(pb.xy.0, pb.xy.1);
        painter.line_segment([a, b], Stroke::new(1.7, TEAL));
        painter.circle_filled(a, 3.0, TEAL);
        painter.circle_filled(b, 3.0, TEAL);
        let mid = Pos2::new((a.x + b.x) * 0.5, (a.y + b.y) * 0.5 - 8.0);
        let text = view.text(meter.id);
        painter.text(mid, egui::Align2::CENTER_BOTTOM, text, FontId::monospace(12.0), TEAL);
    }
    clicked
}
