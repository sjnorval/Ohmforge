use egui::{Align2, FontId, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use super::simulation::{GPIO, LED_ON_MA, LED_REF, R_SERIES, RESISTOR_REF};
use super::{LED_RED, TEAL, panel};

pub(super) fn show(ui: &mut Ui, current_ma: f64) {
    let on = current_ma > LED_ON_MA;
    panel().show(ui, |ui| {
        ui.label(RichText::new("Schematic").strong());
        draw(ui, on);
        ui.horizontal(|ui| {
            ui.label(RichText::new(if on { "LED on" } else { "LED off" }).strong());
            ui.separator();
            let current = if current_ma.abs() < 0.005 { 0.0 } else { current_ma };
            ui.monospace(format!("{current:.2} mA"));
        });
    });
}

fn draw(ui: &mut Ui, on: bool) {
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), 215.0), Sense::hover());
    let description = format!(
        "ESP32-C3 GPIO{GPIO} connects through {RESISTOR_REF} ({R_SERIES:.0} ohms) to the {LED_REF} anode. The cathode returns to ESP32-C3 ground."
    );
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &description));

    // Keep symbols a fixed size and centre the schematic when it has room to grow.
    let left = response.rect.center().x - 200.0;
    let point = |x, y| pos2(left + x, response.rect.top() + y);
    let text = ui.visuals().text_color();
    let muted = ui.visuals().weak_text_color();
    let wire = Stroke::new(1.7, muted);
    let signal = if on { Stroke::new(2.0, TEAL) } else { wire };
    let led = if on { Stroke::new(2.4, LED_RED) } else { wire };
    let line = |points: &[(f32, f32)], stroke| {
        painter.add(Shape::line(points.iter().map(|&(x, y)| point(x, y)).collect(), stroke));
    };
    let label = |x, y, align, name: &str, color| {
        painter.text(point(x, y), align, name, FontId::monospace(12.0), color);
    };

    let chip = Rect::from_min_max(point(18.0, 49.0), point(120.0, 149.0));
    painter.rect_stroke(chip, 4.0, wire, StrokeKind::Inside);
    label(18.0, 34.0, Align2::LEFT_CENTER, "U1 / ESP32-C3", text);
    label(111.0, 82.0, Align2::RIGHT_CENTER, &format!("GPIO{GPIO}"), text);
    label(111.0, 133.0, Align2::RIGHT_CENTER, "GND", muted);

    // GPIO output to the resistor, then from the resistor to the LED anode.
    line(
        &[
            (120.0, 82.0),
            (157.0, 82.0),
            (162.0, 74.0),
            (170.0, 90.0),
            (178.0, 74.0),
            (186.0, 90.0),
            (194.0, 74.0),
            (202.0, 90.0),
            (207.0, 82.0),
            (267.0, 82.0),
        ],
        signal,
    );
    label(182.0, 34.0, Align2::CENTER_CENTER, RESISTOR_REF, text);
    label(182.0, 53.0, Align2::CENTER_CENTER, &format!("{R_SERIES:.0} Ω"), muted);

    if on {
        painter.circle_filled(point(279.0, 82.0), 29.0, LED_RED.gamma_multiply(0.13));
        painter.circle_filled(point(279.0, 82.0), 22.0, LED_RED.gamma_multiply(0.18));
    }
    line(&[(267.0, 70.0), (291.0, 82.0), (267.0, 94.0), (267.0, 70.0)], led);
    line(&[(291.0, 69.0), (291.0, 95.0)], led);
    // The arrowheads point away from the diode.
    for (x, y) in [(280.0, 67.0), (291.0, 70.0)] {
        line(&[(x, y), (x + 11.0, y - 11.0), (x + 5.0, y - 10.0)], led);
        line(&[(x + 10.0, y - 5.0), (x + 11.0, y - 11.0)], led);
    }
    label(276.0, 34.0, Align2::CENTER_CENTER, LED_REF, text);
    label(258.0, 106.0, Align2::CENTER_CENTER, "A", muted);
    label(301.0, 106.0, Align2::CENTER_CENTER, "K", muted);
    label(279.0, 125.0, Align2::CENTER_CENTER, "Red LED", text);

    // The cathode and MCU ground pin share the return net.
    line(
        &[
            (291.0, 82.0),
            (382.0, 82.0),
            (382.0, 154.0),
            (142.0, 154.0),
            (142.0, 133.0),
            (120.0, 133.0),
        ],
        wire,
    );
    painter.circle_filled(point(222.0, 154.0), 3.0, muted);
    line(&[(222.0, 154.0), (222.0, 174.0)], wire);
    for (y, half_width) in [(174.0, 11.0), (179.0, 7.0), (184.0, 3.0)] {
        line(&[(222.0 - half_width, y), (222.0 + half_width, y)], wire);
    }
    label(222.0, 201.0, Align2::CENTER_CENTER, "GND", muted);
}
