use egui::{Align2, FontId, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{GPIO_LED, GPIO_POT, LED_REF, POT_REF, RESISTOR_REF};
use crate::theme::{LED_RED, TEAL, panel};

pub(crate) fn show(ui: &mut Ui, current_ma: f64, brightness: f32, position: Option<f64>) {
    panel().show(ui, |ui| {
        ui.label(RichText::new("Schematic").strong());
        draw(ui, brightness, position);
        ui.horizontal(|ui| {
            let pct = (brightness * 100.0).round() as i32;
            ui.label(RichText::new(format!("LED {pct}%")).strong());
            ui.separator();
            let current = if current_ma.abs() < 0.005 { 0.0 } else { current_ma };
            ui.monospace(format!("{current:.2} mA"));
            if let Some(pos) = position {
                ui.separator();
                ui.monospace(format!("RV1 {:.0}%", pos * 100.0));
            }
        });
    });
}

fn draw(ui: &mut Ui, brightness: f32, position: Option<f64>) {
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), 268.0), Sense::hover());
    let description = if position.is_some() {
        format!(
            "ESP32-C3 GPIO{GPIO_LED} connects through {RESISTOR_REF} (330 ohms) to the {LED_REF} anode. {POT_REF}, a 10 kilohm potentiometer, sits between ground and 3.3 volts with its wiper on GPIO{GPIO_POT}."
        )
    } else {
        format!(
            "ESP32-C3 GPIO{GPIO_LED} connects through {RESISTOR_REF} (330 ohms) to the {LED_REF} anode. The cathode returns to ESP32-C3 ground. GPIO{GPIO_LED} is driven by the LEDC PWM timer."
        )
    };
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, &description));

    let left = response.rect.center().x - 200.0;
    let point = |x, y| pos2(left + x, response.rect.top() + y);
    let text = ui.visuals().text_color();
    let muted = ui.visuals().weak_text_color();
    let wire = Stroke::new(1.7, muted);
    let on = brightness > 0.02;
    let signal = if on { Stroke::new(2.0, TEAL) } else { wire };
    let led = if on {
        Stroke::new(2.4, LED_RED.gamma_multiply(0.45 + 0.55 * brightness))
    } else {
        wire
    };
    let line = |points: &[(f32, f32)], stroke| {
        painter.add(Shape::line(points.iter().map(|&(x, y)| point(x, y)).collect(), stroke));
    };
    let label = |x, y, align, name: &str, color| {
        painter.text(point(x, y), align, name, FontId::monospace(12.0), color);
    };

    let chip = Rect::from_min_max(point(18.0, 49.0), point(120.0, 149.0));
    painter.rect_stroke(chip, 4.0, wire, StrokeKind::Inside);
    label(18.0, 34.0, Align2::LEFT_CENTER, "U1 / ESP32-C3", text);
    label(111.0, 82.0, Align2::RIGHT_CENTER, &format!("GPIO{GPIO_LED}"), text);
    if position.is_some() {
        label(111.0, 112.0, Align2::RIGHT_CENTER, &format!("GPIO{GPIO_POT}"), text);
    }
    label(111.0, 133.0, Align2::RIGHT_CENTER, "GND", muted);

    // GPIO8 to the resistor, then to the LED anode.
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
    label(182.0, 53.0, Align2::CENTER_CENTER, "330 Ω", muted);

    if on {
        painter.circle_filled(
            point(279.0, 82.0),
            29.0,
            LED_RED.gamma_multiply(0.08 + 0.22 * brightness),
        );
        painter.circle_filled(
            point(279.0, 82.0),
            22.0,
            LED_RED.gamma_multiply(0.10 + 0.28 * brightness),
        );
    }
    line(&[(267.0, 70.0), (291.0, 82.0), (267.0, 94.0), (267.0, 70.0)], led);
    line(&[(291.0, 69.0), (291.0, 95.0)], led);
    for (x, y) in [(280.0, 67.0), (291.0, 70.0)] {
        line(&[(x, y), (x + 11.0, y - 11.0), (x + 5.0, y - 10.0)], led);
        line(&[(x + 10.0, y - 5.0), (x + 11.0, y - 11.0)], led);
    }
    label(276.0, 34.0, Align2::CENTER_CENTER, LED_REF, text);
    label(258.0, 106.0, Align2::CENTER_CENTER, "A", muted);
    label(301.0, 106.0, Align2::CENTER_CENTER, "K", muted);
    label(279.0, 125.0, Align2::CENTER_CENTER, "Red LED", text);

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

    if let Some(pos) = position {
        // Track between 3.3 V (top) and ground (bottom); the wiper slides with position.
        let track = Rect::from_min_max(point(52.0, 168.0), point(72.0, 228.0));
        painter.rect_stroke(track, 2.0, wire, StrokeKind::Outside);
        line(&[(62.0, 168.0), (62.0, 154.0)], wire);
        // 3.3 V rail to the top of the track (shown as a short stub and label).
        line(&[(62.0, 168.0), (62.0, 158.0), (28.0, 158.0)], wire);
        label(26.0, 158.0, Align2::RIGHT_CENTER, "3.3 V", text);
        line(&[(62.0, 228.0), (62.0, 238.0)], wire);
        label(62.0, 258.0, Align2::CENTER_CENTER, "GND", muted);
        label(52.0, 178.0, Align2::RIGHT_CENTER, "3", muted);
        label(52.0, 220.0, Align2::RIGHT_CENTER, "1", muted);
        // position 0 is pin 1 (bottom), 1 is pin 3 (top).
        let wy = 228.0 - 60.0 * pos as f32;
        let wiper = point(72.0, wy);
        painter.line_segment([point(52.0, wy), point(86.0, wy)], Stroke::new(2.0, TEAL));
        painter.circle_filled(wiper, 4.0, TEAL);
        line(
            &[(86.0, wy), (132.0, wy), (132.0, 112.0), (120.0, 112.0)],
            Stroke::new(1.7, TEAL),
        );
        label(98.0, 238.0, Align2::CENTER_CENTER, POT_REF, text);
        label(98.0, 168.0, Align2::LEFT_CENTER, "10 kΩ", muted);
    }
}
