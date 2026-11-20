//! Shared hover helpers and PSU / button symbols drawn in the existing style.

use egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Ui};

use crate::theme::TEAL;

/// Invisible hover target with a tooltip, no extra drawing.
pub fn hover_tooltip(ui: &Ui, rect: Rect, id: &'static str, text: &str) {
    let r = ui.interact(rect, ui.scope_id().with(id), Sense::hover());
    r.on_hover_text(text);
}

/// Hold-to-press hit target. Returns whether the pointer is down on the rect.
pub fn hold_press(ui: &Ui, rect: Rect, id: &'static str, tooltip: &str) -> bool {
    let r = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
    r.on_hover_text(tooltip).is_pointer_button_down_on()
}

/// A compact bench-supply rectangle in the existing muted/teal schematic language.
///
/// `origin` is the top-left of the box. Returns the positive-terminal point
/// and the body rect (for tooltips).
pub fn draw_psu(
    painter: &egui::Painter,
    origin: Pos2,
    volts: f64,
    amps: f64,
    on: bool,
    mode: &str,
    plus_left: bool,
    muted: Color32,
) -> (Pos2, Rect) {
    let box_r = Rect::from_min_size(origin, egui::vec2(78.0, 52.0));
    let stroke = Stroke::new(1.7, muted);
    painter.rect_stroke(box_r, 2.0, stroke, egui::StrokeKind::Outside);
    let font = FontId::monospace(12.0);
    painter.text(
        Pos2::new(box_r.center().x, box_r.top() + 11.0),
        egui::Align2::CENTER_CENTER,
        "PSU",
        font.clone(),
        muted,
    );
    let label = if on {
        format!("{volts:.2} V  {mode}")
    } else {
        "OFF".into()
    };
    painter.text(
        Pos2::new(box_r.center().x, box_r.top() + 26.0),
        egui::Align2::CENTER_CENTER,
        label,
        font.clone(),
        if on { TEAL } else { muted },
    );
    if on {
        let ma = amps * 1e3;
        let i_label = if ma.abs() >= 100.0 {
            format!("{ma:.0} mA")
        } else {
            format!("{ma:.2} mA")
        };
        painter.text(
            Pos2::new(box_r.center().x, box_r.top() + 40.0),
            egui::Align2::CENTER_CENTER,
            i_label,
            font,
            TEAL,
        );
    }
    let (plus, minus, inward) = if plus_left {
        (
            Pos2::new(box_r.left(), box_r.top() + 14.0),
            Pos2::new(box_r.left(), box_r.bottom() - 10.0),
            6.0,
        )
    } else {
        (
            Pos2::new(box_r.right(), box_r.top() + 14.0),
            Pos2::new(box_r.right(), box_r.bottom() - 10.0),
            -6.0,
        )
    };
    painter.line_segment([Pos2::new(plus.x + inward, plus.y), plus], Stroke::new(1.7, TEAL));
    painter.text(
        Pos2::new(plus.x + inward * 1.6, plus.y),
        if plus_left {
            egui::Align2::LEFT_CENTER
        } else {
            egui::Align2::RIGHT_CENTER
        },
        "+",
        FontId::monospace(11.0),
        TEAL,
    );
    painter.line_segment([Pos2::new(minus.x + inward, minus.y), minus], stroke);
    (plus, box_r)
}

/// A 6 mm tactile switch, open or closed, in the same stroke language.
///
/// Returns the body rect (for hit testing) and the two terminal points
/// (left = GPIO side, right = ground side).
pub fn draw_switch(painter: &egui::Painter, origin: Pos2, closed: bool, muted: Color32) -> (Rect, Pos2, Pos2) {
    let body = Rect::from_min_size(origin, egui::vec2(56.0, 36.0));
    let stroke = Stroke::new(1.7, muted);
    painter.text(
        Pos2::new(body.center().x, body.top() + 2.0),
        egui::Align2::CENTER_TOP,
        "SW1",
        FontId::monospace(12.0),
        muted,
    );
    let mid_y = body.center().y + 4.0;
    let left = Pos2::new(body.left() + 8.0, mid_y);
    let right = Pos2::new(body.right() - 8.0, mid_y);
    let term_l = Pos2::new(body.left(), mid_y);
    let term_r = Pos2::new(body.right(), mid_y);
    painter.circle_stroke(left, 2.4, stroke);
    painter.circle_stroke(right, 2.4, stroke);
    painter.line_segment([term_l, Pos2::new(left.x - 2.4, mid_y)], stroke);
    painter.line_segment([Pos2::new(right.x + 2.4, mid_y), term_r], stroke);
    if closed {
        painter.line_segment(
            [Pos2::new(left.x + 3.0, mid_y), Pos2::new(right.x - 3.0, mid_y)],
            Stroke::new(2.0, TEAL),
        );
    } else {
        painter.line_segment(
            [Pos2::new(left.x + 3.0, mid_y), Pos2::new(right.x - 4.0, mid_y - 10.0)],
            stroke,
        );
    }
    (body, term_l, term_r)
}
