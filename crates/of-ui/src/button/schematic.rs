use egui::{Align2, FontId, Rect, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};

use super::{GPIO_BTN, GPIO_LED, LED_REF, RESISTOR_REF, SWITCH_REF};
use crate::meters_ui::{MeterView, ProbePoint, SchematicHit, meter_layer};
use crate::schematic::{draw_psu, draw_switch, hold_press, hover_tooltip};
use crate::supply::{self, SupplyStatus};
use crate::theme::{LED_RED, TEAL, panel};
use crate::wiring::{self, Point};

/// Draw the button board. Returns whether SW1 is being held down, plus any probe click.
pub(crate) fn show(
    ui: &mut Ui,
    current_ma: f64,
    brightness: f32,
    closed: bool,
    supply: SupplyStatus,
    meters: &MeterView<'_>,
) -> SchematicHit {
    let mut hit = SchematicHit::default();
    panel().show(ui, |ui| {
        ui.label(RichText::new("Schematic").strong());
        hit = draw(ui, brightness, closed, supply, meters);
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
            ui.separator();
            ui.monospace(if hit.button { "SW1 held" } else { "SW1 open" });
        });
    });
    hit
}

fn draw(ui: &mut Ui, brightness: f32, closed: bool, supply: SupplyStatus, meters: &MeterView<'_>) -> SchematicHit {
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), 268.0), Sense::hover());
    let description = format!(
        "ESP32-C3 GPIO{GPIO_LED} connects through {RESISTOR_REF} (330 ohms) to the {LED_REF} anode. {SWITCH_REF} pulls GPIO{GPIO_BTN} to ground when held."
    );
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

    let board = wiring::button_board();
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

    let chip = Rect::from_min_max(point(18.0, 49.0), point(120.0, 149.0));
    painter.rect_stroke(chip, 4.0, wire, StrokeKind::Inside);
    label(18.0, 34.0, Align2::LEFT_CENTER, "U1 / ESP32-C3", text);
    label(111.0, 82.0, Align2::RIGHT_CENTER, &format!("GPIO{GPIO_LED}"), text);
    label(111.0, 112.0, Align2::RIGHT_CENTER, &format!("GPIO{GPIO_BTN}"), text);
    label(111.0, 133.0, Align2::RIGHT_CENTER, "GND", muted);
    label(27.0, 64.0, Align2::LEFT_CENTER, "VDD", text);

    let gpio8_pin = Point::new(120.0, 82.0);
    let r2_p1 = Point::new(157.0, 82.0);
    let r2_p2 = Point::new(207.0, 82.0);
    let mut signal_pts: Vec<Point> = Vec::new();
    if let Some(route) = board.net("gpio8").and_then(|n| n.route.as_ref())
        && let Some(poly) = route.polylines.first()
    {
        signal_pts.extend(orient(poly, gpio8_pin));
    }
    let zigzag = wiring::r1_zigzag();
    if signal_pts.last().is_some_and(|p| p.distance(r2_p1) <= 1e-4) {
        signal_pts.extend(zigzag.iter().copied().skip(1));
    } else {
        signal_pts.extend(zigzag.iter().copied());
    }
    if let Some(route) = board.net("led_a").and_then(|n| n.route.as_ref())
        && let Some(poly) = route.polylines.first()
    {
        let led = orient(poly, r2_p2);
        if signal_pts.last().is_some_and(|p| p.distance(r2_p2) <= 1e-4) {
            signal_pts.extend(led.into_iter().skip(1));
        } else {
            signal_pts.extend(led);
        }
    }
    paint_poly(&signal_pts, signal);
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

    let (body, _term_l, _term_r) = draw_switch(&painter, point(130.0, 168.0), closed, muted);
    let btn_stroke = if closed { Stroke::new(1.7, TEAL) } else { wire };
    if let Some(route) = board.net("gpio9").and_then(|n| n.route.as_ref()) {
        for poly in &route.polylines {
            for piece in wiring::poly_with_hops(poly, &route.hops) {
                paint_poly(&piece, btn_stroke);
            }
        }
    }

    let (plus, minus_pt, psu) = draw_psu(
        &painter,
        point(18.0, 210.0),
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
    let _ = (plus, minus_pt);
    label(116.0, 262.0, Align2::CENTER_CENTER, "GND", muted);

    hover_tooltip(ui, chip, "u1", "U1 · ESP32-C3");
    hover_tooltip(
        ui,
        Rect::from_min_max(point(157.0, 70.0), point(207.0, 94.0)),
        "r2",
        "R2 · 330 Ω",
    );
    hover_tooltip(
        ui,
        Rect::from_min_max(point(255.0, 60.0), point(310.0, 130.0)),
        "d1",
        "D1 · red LED 0603",
    );
    hover_tooltip(ui, psu, "psu", "Bench supply feeding the VDD rail");
    let button = hold_press(ui, body, "sw1", "SW1 · hold to close (GPIO9 to ground)");

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
            id: "gpio9",
            node: "gpio9",
            label: "GPIO9",
            xy: (120.0, 112.0),
        },
        ProbePoint {
            id: "sw1_a",
            node: "gpio9",
            label: "SW1 (GPIO9)",
            xy: (130.0, 190.0),
        },
        ProbePoint {
            id: "sw1_b",
            node: "0",
            label: "SW1 (GND)",
            xy: (186.0, 190.0),
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
    SchematicHit { button, probe }
}
