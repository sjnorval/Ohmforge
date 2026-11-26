//! The 555 demos' schematic painter: every part of the routed file
//! schematic drawn from its body and pins (no part is looked up by a fixed
//! id, so a file without one still paints), the label rows from the file's
//! label keepouts, and the LED lit with the output.

use egui::{Align2, Color32, FontId, Pos2, RichText, Sense, Shape, Stroke, StrokeKind, Ui, pos2, vec2};
use of_circuit::CircuitFile;

use crate::meters::ProbeRef;
use crate::meters_ui::{MeterView, ProbePoint, SchematicHit, meter_layer};
use crate::schematic::draw_psu;
use crate::supply::{self, SupplyStatus};
use crate::theme::{LED_RED, TEAL, panel};
use crate::wiring::{self, Component, Direction, Point};

/// What a body draws as, from its file part.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Symbol {
    Timer,
    /// The ESP32-C3 (a labelled box).
    Mcu,
    /// A linear regulator (a labelled box).
    Regulator,
    Resistor,
    Pot,
    Capacitor,
    Led,
    Diode,
    Switch,
    Supply,
    Ground,
    Box,
}

/// The symbol for a component of `file` drawn as `id` (a symbol table's
/// ground is [`Symbol::Ground`]).
pub(crate) fn symbol_of(file: &CircuitFile, comp: &Component) -> Symbol {
    let Some(c) = file.components.iter().find(|c| {
        c.schematic
            .as_ref()
            .and_then(|s| s.id.clone())
            .unwrap_or_else(|| c.reference().to_lowercase())
            == comp.id
    }) else {
        return if file.symbols.iter().any(|y| y.reference() == comp.reference) {
            Symbol::Ground
        } else {
            Symbol::Box
        };
    };
    let part = c.part.get_ref().as_str();
    match part {
        p if p.contains("ne555") => Symbol::Timer,
        p if p.contains("esp32") => Symbol::Mcu,
        p if p.contains("ams1117") || p.contains("regulator") => Symbol::Regulator,
        p if p.contains("led") => Symbol::Led,
        p if p.contains("1n4148") || p.contains("diode") => Symbol::Diode,
        p if p.contains("ptv") || p.contains("pot") => Symbol::Pot,
        p if p.contains("resistor") => Symbol::Resistor,
        p if p.contains("capacitor") => Symbol::Capacitor,
        p if p.contains("pushbutton") || p.contains("switch") => Symbol::Switch,
        p if p.contains("psu") => Symbol::Supply,
        _ => Symbol::Box,
    }
}

/// Draw a 555 board; `board` is the routed schematic of `file`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn show(
    ui: &mut Ui,
    file: &CircuitFile,
    board: &wiring::Schematic,
    current_ma: f64,
    brightness: f32,
    closed: bool,
    supply: SupplyStatus,
    meters: &MeterView<'_>,
) -> SchematicHit {
    let mut hit = SchematicHit::default();
    panel().show(ui, |ui| {
        ui.label(RichText::new("Schematic").strong());
        hit.probe = draw(ui, file, board, brightness, closed, supply, meters);
        ui.horizontal(|ui| {
            let pct = (brightness * 100.0).round() as i32;
            ui.label(
                RichText::new(if pct == 0 {
                    "LED off".into()
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

/// A meter probe point: one per routed pin, on its net's solver node (the
/// ground net is node `0`, as on the other demos).
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Probe {
    pub id: String,
    pub node: String,
    pub label: String,
    pub at: Point,
}

/// The probe points of `board` (the routed schematic of `file`), in net order.
pub(crate) fn probe_points(file: &CircuitFile, board: &wiring::Schematic) -> Vec<Probe> {
    let ground = file.circuit.ground.get_ref().as_str();
    let mut out = Vec::new();
    for net in &board.nets {
        let node = if net.kind == wiring::NetKind::Ground || net.id.eq_ignore_ascii_case(ground) {
            "0".to_owned()
        } else {
            net.id.clone()
        };
        for t in &net.terminals {
            let Some(c) = board.component(&t.component) else {
                continue;
            };
            let Some(p) = c.pins.iter().find(|p| p.id == t.pin) else {
                continue;
            };
            out.push(Probe {
                id: format!("{}.{}", c.id, p.id),
                node: node.clone(),
                label: format!("{} {} \u{00b7} {}", c.reference, p.name, net.name),
                at: p.position,
            });
        }
    }
    out
}

fn draw(
    ui: &mut Ui,
    file: &CircuitFile,
    board: &wiring::Schematic,
    brightness: f32,
    closed: bool,
    supply: SupplyStatus,
    meters: &MeterView<'_>,
) -> Option<ProbeRef> {
    let extent = board
        .components
        .iter()
        .fold((0.0f32, 0.0f32), |(w, h), c| (w.max(c.body.max.x), h.max(c.body.max.y)));
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), extent.1 + 40.0), Sense::hover());
    response.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Image, true, file.circuit.name.as_str()));
    let left = (response.rect.center().x - (extent.0 + 20.0) * 0.5).max(response.rect.left());
    let top = response.rect.top() + 4.0;
    let pt = |p: Point| pos2(left + p.x, top + p.y);
    let xy = |x: f32, y: f32| pos2(left + x, top + y);
    let text = ui.visuals().text_color();
    let muted = ui.visuals().weak_text_color();
    let wire = Stroke::new(1.7, muted);
    let on = brightness > 0.02;
    let lit = Stroke::new(2.0, TEAL);

    // Nets: the supply rail teal, the output path teal while the LED is lit.
    for net in &board.nets {
        let Some(route) = &net.route else { continue };
        let stroke = match net.id.as_str() {
            "vcc" => lit,
            "out" | "led_a" if on => lit,
            _ => wire,
        };
        for poly in &route.polylines {
            for piece in wiring::poly_with_hops(poly, &route.hops) {
                if piece.len() >= 2 {
                    painter.add(Shape::line(piece.iter().map(|p| pt(*p)).collect(), stroke));
                }
            }
        }
        for j in &route.junctions {
            painter.circle_filled(pt(*j), 3.0, if stroke == lit { TEAL } else { muted });
        }
    }

    for comp in &board.components {
        let b = comp.body;
        let (min, max) = (pt(b.min), pt(b.max));
        let centre = pos2((min.x + max.x) * 0.5, (min.y + max.y) * 0.5);
        let vertical = (max.y - min.y) >= (max.x - min.x);
        let symbol = symbol_of(file, comp);
        match symbol {
            Symbol::Timer | Symbol::Mcu | Symbol::Regulator | Symbol::Box => {
                painter.rect_stroke(egui::Rect::from_min_max(min, max), 4.0, wire, StrokeKind::Inside);
                painter.text(
                    centre - vec2(0.0, 8.0),
                    Align2::CENTER_CENTER,
                    &comp.reference,
                    FontId::monospace(13.0),
                    text,
                );
                painter.text(
                    centre + vec2(0.0, 8.0),
                    Align2::CENTER_CENTER,
                    match symbol {
                        Symbol::Timer => "NE555",
                        Symbol::Mcu => "ESP32-C3",
                        // A regulator body is short: its GND pin label sits
                        // where a subtitle would, so the file's label above
                        // names the regulator.
                        Symbol::Regulator => "",
                        _ => "",
                    },
                    FontId::monospace(12.0),
                    muted,
                );
                for p in &comp.pins {
                    let (off, align) = match p.direction {
                        Direction::West => (vec2(5.0, 0.0), Align2::LEFT_CENTER),
                        Direction::East => (vec2(-5.0, 0.0), Align2::RIGHT_CENTER),
                        Direction::North => (vec2(0.0, 8.0), Align2::CENTER_CENTER),
                        Direction::South => (vec2(0.0, -8.0), Align2::CENTER_CENTER),
                    };
                    painter.text(pt(p.position) + off, align, &p.name, FontId::monospace(10.0), muted);
                }
            }
            Symbol::Resistor | Symbol::Pot => {
                let (a, z) = if vertical {
                    (pos2(centre.x, min.y), pos2(centre.x, max.y))
                } else {
                    (pos2(min.x, centre.y), pos2(max.x, centre.y))
                };
                let n = 6;
                let amp = 6.0;
                let mut pts = vec![a];
                for k in 0..n {
                    let f = (k as f32 + 0.5) / n as f32;
                    let s = if k % 2 == 0 { amp } else { -amp };
                    let m = a + (z - a) * f;
                    pts.push(if vertical { m + vec2(s, 0.0) } else { m + vec2(0.0, s) });
                }
                pts.push(z);
                painter.add(Shape::line(pts, wire));
                if let Some(w) = comp.pins.iter().find(|p| p.id == "wiper") {
                    // The wiper arrow points at the track from the slider.
                    let tip = pos2(centre.x + amp + 2.0, pt(w.position).y);
                    painter.line_segment([pt(w.position), tip], wire);
                    painter.add(Shape::line(
                        vec![tip + vec2(6.0, -4.0), tip, tip + vec2(6.0, 4.0)],
                        wire,
                    ));
                }
            }
            Symbol::Capacitor => {
                let (gap, half) = (4.0, 10.0);
                if vertical {
                    painter.line_segment([pos2(centre.x, min.y), pos2(centre.x, centre.y - gap)], wire);
                    painter.line_segment([pos2(centre.x, centre.y + gap), pos2(centre.x, max.y)], wire);
                    for dy in [-gap, gap] {
                        painter.line_segment(
                            [
                                pos2(centre.x - half, centre.y + dy),
                                pos2(centre.x + half, centre.y + dy),
                            ],
                            wire,
                        );
                    }
                } else {
                    for dx in [-gap, gap] {
                        painter.line_segment(
                            [
                                pos2(centre.x + dx, centre.y - half),
                                pos2(centre.x + dx, centre.y + half),
                            ],
                            wire,
                        );
                    }
                }
            }
            Symbol::Led | Symbol::Diode => {
                let led = Symbol::Led == symbol_of(file, comp);
                let stroke = if led && on {
                    Stroke::new(2.4, LED_RED.gamma_multiply(0.45 + 0.55 * brightness))
                } else {
                    wire
                };
                if led && on {
                    painter.circle_filled(centre, 22.0, LED_RED.gamma_multiply(0.10 + 0.28 * brightness));
                }
                // Anode to cathode, from the pins.
                let pin = |name: &str| {
                    comp.pins
                        .iter()
                        .find(|p| p.name == name || p.id == name)
                        .map(|p| pt(p.position))
                };
                let (Some(an), Some(ka)) = (pin("A"), pin("K")) else {
                    continue;
                };
                let dir = (ka - an).normalized();
                let side = vec2(-dir.y, dir.x);
                let (base, tip) = (centre - dir * 7.0, centre + dir * 7.0);
                painter.line_segment([an, base], wire);
                painter.line_segment([tip, ka], wire);
                painter.add(Shape::convex_polygon(
                    vec![base + side * 8.0, tip, base - side * 8.0],
                    Color32::TRANSPARENT,
                    stroke,
                ));
                painter.line_segment([tip + side * 8.0, tip - side * 8.0], stroke);
                if led {
                    for o in [0.0, 6.0] {
                        let s = centre + side * (12.0 + o) + dir * (o - 3.0);
                        painter.add(Shape::line(vec![s, s + side * 8.0 + dir * 4.0], stroke));
                    }
                }
            }
            Symbol::Switch => {
                let (a, z) = (pos2(centre.x, min.y), pos2(centre.x, max.y));
                painter.line_segment([a, a + vec2(0.0, 10.0)], wire);
                painter.line_segment([z - vec2(0.0, 10.0), z], wire);
                painter.circle_stroke(a + vec2(0.0, 12.0), 2.0, wire);
                painter.circle_stroke(z - vec2(0.0, 12.0), 2.0, wire);
                let x = if closed { centre.x - 4.0 } else { centre.x - 12.0 };
                let stroke = if closed { lit } else { wire };
                painter.line_segment([pos2(x, a.y + 8.0), pos2(x, z.y - 8.0)], stroke);
                painter.line_segment([pos2(x, centre.y), pos2(x - 6.0, centre.y)], stroke);
            }
            Symbol::Supply => {
                let _ = draw_psu(
                    &painter,
                    min,
                    supply.volts,
                    supply.display_amps(),
                    supply.mode != supply::SupplyMode::Off,
                    supply::mode_label(supply.mode),
                    false,
                    muted,
                );
            }
            Symbol::Ground => {
                let x = centre.x;
                for (dy, hw) in [(0.0, 11.0), (5.0, 7.0), (10.0, 3.0)] {
                    painter.line_segment([pos2(x - hw, min.y + dy), pos2(x + hw, min.y + dy)], wire);
                }
            }
        }
    }

    // Label rows: the file's short keepouts carry the text drawn in them.
    for k in &file.schematic.keepouts {
        if k.rect[3] - k.rect[1] > 16.0 {
            continue;
        }
        let Some(note) = k.note.as_deref() else { continue };
        let is_ref = board.components.iter().any(|c| c.reference == note);
        painter.text(
            xy((k.rect[0] + k.rect[2]) * 0.5, (k.rect[1] + k.rect[3]) * 0.5),
            Align2::CENTER_CENTER,
            note,
            FontId::monospace(12.0),
            if is_ref { text } else { muted },
        );
    }
    let probes = probe_points(file, board);
    let points: Vec<ProbePoint<'_>> = probes
        .iter()
        .map(|p| ProbePoint {
            id: &p.id,
            node: &p.node,
            label: &p.label,
            xy: (p.at.x, p.at.y),
        })
        .collect();
    let _: Pos2 = xy(0.0, 0.0);
    meter_layer(ui, xy, &painter, &points, meters, muted)
}

#[cfg(test)]
mod tests {
    use super::probe_points;
    use crate::kind::DemoKind;
    use crate::session_sim::{SessionSim, StartOptions};
    use of_instruments::{Multimeter, Reading};

    /// The picker lists every demo once, the 555 ones included, and the
    /// painter has a symbol for every part of the 555 boards.
    #[test]
    fn the_picker_offers_the_555_demos_and_they_paint() {
        let picker = crate::kind::PICKER;
        for (i, k) in picker.iter().enumerate() {
            assert!(!picker[..i].contains(k), "{k:?} listed twice");
        }
        for kind in [
            DemoKind::Ne555Astable,
            DemoKind::Ne555Monostable,
            DemoKind::Ne555Pwm,
            DemoKind::Clock555,
        ] {
            assert!(picker.contains(&kind), "{kind:?} is not in the picker");
            let file = crate::session_sim::parsed(kind).unwrap();
            let board = crate::boards::board(kind, Some(0.5));
            for c in &board.components {
                assert_ne!(
                    super::symbol_of(file, c),
                    super::Symbol::Box,
                    "{kind:?} {}",
                    c.reference
                );
            }
        }
    }

    /// Every routed pin of the three 555 files is a probe point: unique ids,
    /// each on a vertex of its own net's route, and a voltmeter from it to
    /// ground reads a finite voltage.
    #[test]
    fn every_555_pin_is_a_probe_point_that_reads() {
        for kind in [
            DemoKind::Ne555Astable,
            DemoKind::Ne555Monostable,
            DemoKind::Ne555Pwm,
            DemoKind::Clock555,
        ] {
            let file = crate::session_sim::parsed(kind).unwrap();
            let board = crate::boards::board(kind, Some(0.5));
            let probes = probe_points(file, &board);
            let mut ids: Vec<&str> = probes.iter().map(|p| p.id.as_str()).collect();
            ids.sort_unstable();
            ids.dedup();
            assert_eq!(ids.len(), probes.len(), "{kind:?} probe ids repeat");
            let terminals: usize = board.nets.iter().map(|n| n.terminals.len()).sum();
            assert_eq!(probes.len(), terminals, "{kind:?}");
            for p in &probes {
                let net = board
                    .nets
                    .iter()
                    .find(|n| n.terminals.iter().any(|t| format!("{}.{}", t.component, t.pin) == p.id))
                    .unwrap();
                let route = net.route.as_ref().unwrap();
                assert!(
                    route.polylines.iter().flatten().any(|v| *v == p.at),
                    "{kind:?} {} is off its route",
                    p.id
                );
            }
            assert!(probes.iter().any(|p| p.node == "0"), "{kind:?} has a ground probe");
            let mut s = SessionSim::start(kind, StartOptions::default()).unwrap();
            s.advance(2e-3).unwrap();
            for p in &probes {
                let r = s.read_meter(&Multimeter::voltmeter(&p.node, "0"));
                assert!(
                    matches!(r, Reading::Volts(v) if v.is_finite()),
                    "{kind:?} {} ({}) reads {r:?}",
                    p.id,
                    p.node
                );
            }
        }
    }
}
