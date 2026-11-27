//! Shared hover helpers and PSU / button symbols drawn in the existing style.

use egui::emath::TSTransform;
use egui::layers::ShapeIdx;
use egui::{
    self, Color32, Context, FontId, Id, LayerId, Painter, Pos2, Rect, Response, Sense, Shape, Stroke, Ui, Vec2, pos2,
    vec2,
};

use crate::theme::{LED_RED, TEAL};

/// Invisible hover target with a tooltip, no extra drawing.
pub fn hover_tooltip(ui: &Ui, rect: Rect, id: &'static str, text: &str) {
    let r = ui.interact(rect, ui.scope_id().with(id), Sense::hover());
    r.on_hover_text(text);
}

/// Largest scale of a fitted schematic, the smallest (only for panels too
/// narrow to read anyway), the tallest it may get and the padding kept
/// around the drawing (points).
pub const FIT_MIN_SCALE: f32 = 0.2;
pub const FIT_MAX_SCALE: f32 = 1.25;
pub const FIT_MAX_HEIGHT: f32 = 640.0;
pub const FIT_PAD: f32 = 10.0;

/// Scale that fits `content` (design points) inside `avail` points of
/// width and `max_height` of height, at most [`FIT_MAX_SCALE`]. No floor
/// short of [`FIT_MIN_SCALE`]: a narrow panel shrinks the drawing rather
/// than clipping it.
pub fn fit_scale(avail: f32, max_height: f32, content: Vec2) -> f32 {
    let s = (avail / content.x.max(1.0))
        .min(max_height / content.y.max(1.0))
        .min(FIT_MAX_SCALE);
    if s.is_finite() { s.max(FIT_MIN_SCALE) } else { 1.0 }
}

/// What the last fitted schematic drew: its painted extent in design
/// points, that extent on screen, the frame it was given and the clip its
/// shapes carry on screen.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FitRecord {
    pub content: Rect,
    pub drawn: Rect,
    pub frame: Rect,
    pub clip: Rect,
}

fn generation_id() -> Id {
    Id::NULL.with("fit-generation")
}

/// The cached extent of drawing `name`, under the current generation (see
/// [`reset_fits`]).
fn content_id(ctx: &Context, name: &str) -> Id {
    let generation = ctx.data(|d| d.get_temp::<u64>(generation_id())).unwrap_or(0);
    Id::NULL.with(("fit-content", generation, name))
}

/// Forget every measured schematic extent (on a demo switch): the next
/// frame measures the new drawing afresh and the one after fits it.
pub fn reset_fits(ctx: &Context) {
    ctx.data_mut(|d| {
        let generation = d.get_temp::<u64>(generation_id()).unwrap_or(0);
        d.insert_temp(generation_id(), generation.wrapping_add(1));
        d.remove::<FitRecord>(last_id());
    });
    ctx.request_repaint();
}

fn last_id() -> Id {
    Id::NULL.with("fit-last")
}

/// The record of the schematic fitted last (tests and diagnostics).
#[cfg_attr(not(test), allow(dead_code))]
pub fn last_fit(ctx: &Context) -> Option<FitRecord> {
    ctx.data(|d| d.get_temp(last_id()))
}

/// A schematic drawn at its design size and scaled to fit the panel. The
/// frame follows the drawing's true painted extent (labels, the PSU box,
/// net names, glow and off-grid parts included), measured from what was
/// painted and kept per drawing (`name`), plus [`FIT_PAD`]. Draw with
/// [`Fit::raw`] points on the returned painter, then call [`Fit::finish`];
/// hit targets and meter overlays use [`Fit::screen`] / [`Fit::screen_rect`]
/// and [`Fit::painter`].
pub struct Fit {
    origin: Pos2,
    translation: Vec2,
    scale: f32,
    start: ShapeIdx,
    layer: LayerId,
    screen: Painter,
    key: Id,
    content: Rect,
    frame: Rect,
}

/// Allocate a fitted canvas for drawing `name`, `design` points at scale 1
/// until its painted extent is known.
pub fn fit_canvas(ui: &mut Ui, name: &str, design: Vec2) -> (Response, Painter, Fit) {
    let key = content_id(ui.ctx(), name);
    let content = ui
        .ctx()
        .data(|d| d.get_temp::<Rect>(key))
        .unwrap_or_else(|| Rect::from_min_size(Pos2::ZERO, design));
    let padded = content.expand(FIT_PAD);
    let avail = ui.available_width();
    let scale = fit_scale(avail, FIT_MAX_HEIGHT, padded.size());
    let (response, screen) = ui.allocate_painter(vec2(avail, padded.height() * scale), Sense::hover());
    let frame = response.rect;
    // Design point d draws at raw `origin + d` and lands on screen at
    // `p0 + (d - padded.min) * scale`.
    let origin = pos2(
        (frame.center().x - padded.width() * scale * 0.5).max(frame.left()),
        frame.top(),
    );
    let translation = origin.to_vec2() * (1.0 - scale) - padded.min.to_vec2() * scale;
    let t = TSTransform::new(translation, scale);
    // Nothing is clipped in design space; `finish` sets the screen clip.
    let raw = screen.with_clip_rect(Rect::EVERYTHING);
    let _ = t;
    let start = raw.add(Shape::Noop);
    let layer = raw.layer_id();
    (
        response,
        raw,
        Fit {
            origin,
            translation,
            scale,
            start,
            layer,
            screen,
            key,
            content,
            frame,
        },
    )
}

impl Fit {
    fn transform(&self) -> TSTransform {
        TSTransform::new(self.translation, self.scale)
    }

    /// A design point on the drawing painter (before [`Self::finish`]).
    pub fn raw(&self, x: f32, y: f32) -> Pos2 {
        pos2(self.origin.x + x, self.origin.y + y)
    }

    /// A design point where it lands on screen.
    pub fn screen(&self, x: f32, y: f32) -> Pos2 {
        self.transform() * self.raw(x, y)
    }

    /// A drawing-painter rect where it lands on screen.
    pub fn screen_rect(&self, rect: Rect) -> Rect {
        self.transform().mul_rect(rect)
    }

    /// Scale everything drawn on `painter` since [`fit_canvas`] into place,
    /// and keep its painted extent for the next frame's fit (asking for
    /// that frame when the extent moved).
    pub fn finish(&self, painter: &Painter) {
        let end = painter.add(Shape::Noop);
        let t = self.transform();
        // The panel's clip on screen. Every scaled shape gets exactly this
        // clip, rather than the drawing painter's clip carried through the
        // transform, so the clip can never sit off the frame.
        let clip = self.screen.clip_rect();
        let ctx = painter.ctx();
        let painted = ctx.graphics_mut(|g| {
            let list = g.entry(self.layer);
            let bbox = list
                .all_entries()
                .skip(self.start.0)
                .take(end.0.saturating_sub(self.start.0))
                .fold(Rect::NOTHING, |r, c| r.union(c.shape.visual_bounding_rect()));
            list.transform_range(self.start, end, t);
            for i in self.start.0..end.0 {
                list.mutate_shape(ShapeIdx(i), |c| c.clip_rect = clip);
            }
            bbox
        });
        if !painted.is_positive() {
            return;
        }
        let content = painted.translate(-self.origin.to_vec2());
        let record = FitRecord {
            content,
            drawn: t.mul_rect(painted),
            frame: self.frame,
            clip,
        };
        ctx.data_mut(|d| {
            d.insert_temp(self.key, content);
            d.insert_temp(last_id(), record);
        });
        if (content.min - self.content.min).length() > 0.5 || (content.max - self.content.max).length() > 0.5 {
            ctx.request_repaint();
        }
    }

    /// A painter in screen points, clipped to the canvas (meter overlays).
    pub fn painter(&self) -> Painter {
        self.screen.clone()
    }
}

/// Brightness below which an LED draws unlit.
pub const LED_ON: f32 = 0.02;

/// The LED symbol and its light, the same on every demo: a triangle from
/// anode to cathode along `dir` with the cathode bar and two emission
/// arrows, 24 points long and centred on `centre`, and a two-ring glow
/// whose strength follows `brightness` (0..=1, from the simulated LED
/// current). Unlit, it draws in the `wire` stroke.
pub fn draw_led(painter: &Painter, centre: Pos2, dir: Vec2, brightness: f32, wire: Stroke) {
    let brightness = brightness.clamp(0.0, 1.0);
    let on = brightness > LED_ON;
    let stroke = if on {
        Stroke::new(2.4, LED_RED.gamma_multiply(0.45 + 0.55 * brightness))
    } else {
        wire
    };
    if on {
        painter.circle_filled(centre, 29.0, LED_RED.gamma_multiply(0.08 + 0.22 * brightness));
        painter.circle_filled(centre, 22.0, LED_RED.gamma_multiply(0.10 + 0.28 * brightness));
    }
    let dir = dir.normalized();
    let up = vec2(dir.y, -dir.x);
    let p = |u: f32, v: f32| centre + dir * u + up * v;
    let line = |pts: Vec<Pos2>| {
        painter.add(Shape::line(pts, stroke));
    };
    line(vec![p(-12.0, 12.0), p(12.0, 0.0), p(-12.0, -12.0), p(-12.0, 12.0)]);
    line(vec![p(12.0, 13.0), p(12.0, -13.0)]);
    // The arrowheads point away from the diode.
    for (u, v) in [(1.0, 15.0), (12.0, 12.0)] {
        line(vec![p(u, v), p(u + 11.0, v + 11.0), p(u + 5.0, v + 10.0)]);
        line(vec![p(u + 10.0, v + 5.0), p(u + 11.0, v + 11.0)]);
    }
}

/// Hold-to-press hit target. Returns whether the pointer is down on the rect.
pub fn hold_press(ui: &Ui, rect: Rect, id: &'static str, tooltip: &str) -> bool {
    let r = ui.interact(rect, ui.scope_id().with(id), Sense::click_and_drag());
    r.on_hover_text(tooltip).is_pointer_button_down_on()
}

/// A compact bench-supply rectangle in the existing muted/teal schematic language.
///
/// `origin` is the top-left of the box. Returns the positive-terminal point,
/// the negative-terminal point, and the body rect (for tooltips).
pub fn draw_psu(
    painter: &egui::Painter,
    origin: Pos2,
    volts: f64,
    amps: f64,
    on: bool,
    mode: &str,
    plus_left: bool,
    muted: Color32,
) -> (Pos2, Pos2, Rect) {
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
        // The minus terminal shares this row: keep the readout clear of it.
        let toward_far_side = if plus_left { 7.0 } else { -7.0 };
        painter.text(
            Pos2::new(box_r.center().x + toward_far_side, box_r.top() + 40.0),
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
    painter.text(
        Pos2::new(minus.x + inward * 1.6, minus.y),
        if plus_left {
            egui::Align2::LEFT_CENTER
        } else {
            egui::Align2::RIGHT_CENTER
        },
        "\u{2212}",
        FontId::monospace(11.0),
        muted,
    );
    (plus, minus, box_r)
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

#[cfg(test)]
mod fit_tests {
    use super::{FIT_MAX_SCALE, FIT_MIN_SCALE, fit_scale};
    use egui::vec2;

    /// The scale fits both the width and the height, up to the cap; narrow
    /// panels shrink the drawing (no 0.55 floor that would clip it).
    #[test]
    fn fit_scale_fits_width_and_height() {
        let d = vec2(400.0, 268.0);
        assert_eq!(fit_scale(400.0, 1000.0, d), 1.0);
        assert_eq!(fit_scale(4000.0, 1000.0, d), FIT_MAX_SCALE);
        assert!((fit_scale(100.0, 1000.0, d) - 0.25).abs() < 1e-6);
        assert_eq!(fit_scale(10.0, 1000.0, d), FIT_MIN_SCALE);
        assert!((fit_scale(800.0, 400.0, vec2(400.0, 800.0)) - 0.5).abs() < 1e-6);
        assert!((FIT_MIN_SCALE..=FIT_MAX_SCALE).contains(&fit_scale(f32::NAN, 1000.0, d)));
    }
}
