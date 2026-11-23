use egui::{Align2, FontId, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use super::simulation::{GPIO, LED_REF, R_SERIES, RESISTOR_REF};
use crate::meters_ui::{MeterView, ProbePoint, SchematicHit, meter_layer};
use crate::schematic::{draw_psu, hover_tooltip};
use crate::supply::{self, SupplyStatus};
use crate::theme::{LED_RED, TEAL, panel};
use crate::wiring::{self, Point};

pub(crate) fn show(
    ui: &mut Ui,
    current_ma: f64,
    brightness: f32,
    supply: SupplyStatus,
    meters: &MeterView<'_>,
) -> SchematicHit {
    let mut hit = SchematicHit::default();
    panel().show(ui, |ui| {
        ui.label(RichText::new("Schematic").strong());
        hit = draw(ui, brightness, supply, meters);
        ui.horizontal(|ui| {
            let pct = (brightness * 100.0).round() as i32;
            ui.label(
                RichText::new(if pct == 0 {
                    "LED off".into()
                } else if pct >= 99 {
                    "LED on".into()
                } else {
                    format!("LED {pct}%")
                })
                .strong(),
            );
            ui.separator();
            let current = if current_ma.abs() < 0.005 { 0.0 } else { current_ma };
            ui.monospace(format!("{current:.2} mA"));
        });
    });
    hit
}

fn draw(ui: &mut Ui, brightness: f32, supply: SupplyStatus, meters: &MeterView<'_>) -> SchematicHit {
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), 268.0), Sense::hover());
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

    let board = wiring::blink_board();
    let paint_poly = |pts: &[Point], stroke: Stroke| {
        if pts.len() < 2 {
            return;
        }
        let screen: Vec<_> = pts.iter().map(|p| point(p.x, p.y)).collect();
        if screen.len() == 2 {
            painter.line_segment([screen[0], screen[1]], stroke);
        } else {
            painter.add(Shape::line(screen, stroke));
        }
    };
    let orient = |pts: &[Point], start: Point| -> Vec<Point> {
        if pts.first().is_some_and(|p| p.distance(start) <= 1e-4) {
            pts.to_vec()
        } else if pts.last().is_some_and(|p| p.distance(start) <= 1e-4) {
            pts.iter().copied().rev().collect()
        } else {
            pts.to_vec()
        }
    };

    let u1 = board.component("u1").expect("u1");
    let chip = Rect::from_min_max(point(u1.body.min.x, u1.body.min.y), point(u1.body.max.x, u1.body.max.y));
    painter.rect_stroke(chip, 4.0, wire, StrokeKind::Inside);
    label(18.0, 34.0, Align2::LEFT_CENTER, "U1 / ESP32-C3", text);
    label(111.0, 82.0, Align2::RIGHT_CENTER, &format!("GPIO{GPIO}"), text);
    label(111.0, 133.0, Align2::RIGHT_CENTER, "GND", muted);

    // GPIO output to the resistor, the R1 symbol zigzag, then the LED anode.
    // Painted as one Shape::line so the historical caps and joins stay put.
    let gpio8_pin = Point::new(120.0, 82.0);
    let r1_p1 = Point::new(157.0, 82.0);
    let r1_p2 = Point::new(207.0, 82.0);
    let mut signal_pts: Vec<Point> = Vec::new();
    if let Some(route) = board.net("gpio8").and_then(|n| n.route.as_ref())
        && let Some(poly) = route.polylines.first()
    {
        signal_pts.extend(orient(poly, gpio8_pin));
    }
    let zigzag = wiring::r1_zigzag();
    if signal_pts.last().is_some_and(|p| p.distance(r1_p1) <= 1e-4) {
        signal_pts.extend(zigzag.iter().copied().skip(1));
    } else {
        signal_pts.extend(zigzag.iter().copied());
    }
    if let Some(route) = board.net("led_a").and_then(|n| n.route.as_ref())
        && let Some(poly) = route.polylines.first()
    {
        let led = orient(poly, r1_p2);
        if signal_pts.last().is_some_and(|p| p.distance(r1_p2) <= 1e-4) {
            signal_pts.extend(led.into_iter().skip(1));
        } else {
            signal_pts.extend(led);
        }
    }
    paint_poly(&signal_pts, signal);
    label(182.0, 34.0, Align2::CENTER_CENTER, RESISTOR_REF, text);
    label(
        182.0,
        53.0,
        Align2::CENTER_CENTER,
        &format!("{R_SERIES:.0} \u{03a9}"),
        muted,
    );

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
    // The arrowheads point away from the diode.
    for (x, y) in [(280.0, 67.0), (291.0, 70.0)] {
        line(&[(x, y), (x + 11.0, y - 11.0), (x + 5.0, y - 10.0)], led);
        line(&[(x + 10.0, y - 5.0), (x + 11.0, y - 11.0)], led);
    }
    label(276.0, 34.0, Align2::CENTER_CENTER, LED_REF, text);
    label(258.0, 106.0, Align2::CENTER_CENTER, "A", muted);
    label(301.0, 106.0, Align2::CENTER_CENTER, "K", muted);
    label(279.0, 125.0, Align2::CENTER_CENTER, "Red LED", text);

    // The cathode and MCU ground pin share the return net. The PSU minus stub
    // is painted later, after the supply symbol, so it sits on top of the box.
    let minus = Point::new(96.0, 252.0);
    let gnd_route = board.net("gnd").and_then(|n| n.route.as_ref());
    if let Some(route) = gnd_route {
        for poly in &route.polylines {
            let is_minus_stub = poly.iter().any(|p| p.distance(minus) <= 1e-4);
            if !is_minus_stub {
                for piece in wiring::poly_with_hops(poly, &route.hops) {
                    paint_poly(&piece, wire);
                }
            }
        }
        for p in &route.junctions {
            painter.circle_filled(point(p.x, p.y), 3.0, muted);
        }
    }
    for (y, half_width) in [(174.0, 11.0), (179.0, 7.0), (184.0, 3.0)] {
        line(&[(222.0 - half_width, y), (222.0 + half_width, y)], wire);
    }
    label(222.0, 201.0, Align2::CENTER_CENTER, "GND", muted);

    // VDD pin on the left of the chip; existing pin labels stay put.
    label(27.0, 64.0, Align2::LEFT_CENTER, "VDD", text);
    let psu = board.component("psu").expect("psu");
    let (plus, minus_pt, psu_rect) = draw_psu(
        &painter,
        point(psu.body.min.x, psu.body.min.y),
        supply.volts,
        supply.amps,
        supply.mode != supply::SupplyMode::Off,
        supply::mode_label(supply.mode),
        false,
        muted,
    );
    if let Some(route) = board.net("vdd").and_then(|n| n.route.as_ref()) {
        for poly in &route.polylines {
            for piece in wiring::poly_with_hops(poly, &route.hops) {
                paint_poly(&piece, Stroke::new(1.7, TEAL));
            }
        }
    }
    if let Some(route) = gnd_route {
        for poly in &route.polylines {
            if poly.iter().any(|p| p.distance(minus) <= 1e-4) {
                for piece in wiring::poly_with_hops(poly, &route.hops) {
                    paint_poly(&piece, wire);
                }
            }
        }
    }
    label(116.0, 262.0, Align2::CENTER_CENTER, "GND", muted);
    let _ = (plus, minus_pt);

    hover_tooltip(ui, chip, "u1", "U1 \u{00b7} ESP32-C3");
    hover_tooltip(
        ui,
        Rect::from_min_max(point(157.0, 70.0), point(207.0, 94.0)),
        "r1",
        "R1 \u{00b7} 330 \u{03a9}",
    );
    hover_tooltip(
        ui,
        Rect::from_min_max(point(255.0, 60.0), point(310.0, 130.0)),
        "d1",
        "D1 \u{00b7} red LED 0603",
    );
    hover_tooltip(ui, psu_rect, "psu", "Bench supply feeding the VDD rail");

    let probes = [
        ProbePoint {
            id: "gpio8",
            node: "gpio8",
            label: "GPIO8",
            xy: (120.0, 82.0),
        },
        ProbePoint {
            id: "led_a",
            node: "led_a",
            label: "LED anode",
            xy: (267.0, 82.0),
        },
        ProbePoint {
            id: "led_k",
            node: "0",
            label: "LED cathode",
            xy: (291.0, 82.0),
        },
        ProbePoint {
            id: "gnd",
            node: "0",
            label: "GND",
            xy: (120.0, 133.0),
        },
        ProbePoint {
            id: "gnd_sym",
            node: "0",
            label: "GND",
            xy: (222.0, 154.0),
        },
        ProbePoint {
            id: "vdd",
            node: "vdd",
            label: "VDD",
            xy: (18.0, 64.0),
        },
        ProbePoint {
            id: "psu_plus",
            node: "vdd",
            label: "PSU +",
            xy: (96.0, 224.0),
        },
        ProbePoint {
            id: "psu_minus",
            node: "0",
            label: "PSU -",
            xy: (96.0, 252.0),
        },
    ];
    let probe = meter_layer(ui, point, &painter, &probes, meters, muted);
    SchematicHit { button: false, probe }
}
