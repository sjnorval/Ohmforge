//! Desktop views of the circuit's UART consoles and logic analysers
//! (engine E5g), in the serial monitor's panel style.

use egui::{RichText, ScrollArea, Sense, Shape, Ui, pos2, vec2};
use of_instruments::LogicAnalyser;

use crate::serial::panel;

/// Time window the analyser view shows, ending now (s).
pub(crate) const LOGIC_WINDOW: f64 = 2e-3;

/// A console as the worker publishes it: title and text so far.
#[derive(Debug, Clone, Default, PartialEq)]
pub(crate) struct ConsoleView {
    pub title: String,
    pub text: String,
    pub framing_errors: usize,
}

/// Draw every console, then every analyser, below the serial monitor.
pub(crate) fn show(ui: &mut Ui, consoles: &[ConsoleView], logic: &[(String, LogicAnalyser)], now: f64) {
    for c in consoles {
        ui.add_space(12.0);
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(&c.title).strong());
                if c.framing_errors > 0 {
                    ui.label(format!("· {} framing error(s)", c.framing_errors));
                }
            });
            ScrollArea::vertical()
                .id_salt(("console", &c.title))
                .max_height(160.0)
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    let text = if c.text.is_empty() {
                        "(idle line)"
                    } else {
                        c.text.as_str()
                    };
                    ui.label(RichText::new(text).monospace());
                });
        });
    }
    for (title, la) in logic {
        ui.add_space(12.0);
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            let clock = la.period.map_or_else(
                || "ideal, exact edges".to_owned(),
                |p| format!("{} MS/s", (1e-6 / p).round()),
            );
            ui.label(RichText::new(format!("{title} · {} ch · {clock}", la.channels.len())).strong());
            let t0 = (now - LOGIC_WINDOW).max(0.0);
            for ch in &la.channels {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:<14}", ch.signal.to_string())).monospace());
                    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width().max(80.0), 22.0), Sense::hover());
                    let Some(mut s) = ch.state_at(t0) else {
                        return;
                    };
                    let span = (now - t0).max(1e-12);
                    let x = |t: f64| rect.left() + ((t - t0) / span) as f32 * rect.width();
                    let y = |s: bool| if s { rect.top() + 3.0 } else { rect.bottom() - 3.0 };
                    let mut pts = vec![pos2(x(t0), y(s))];
                    for &(te, ns) in ch.edges.iter().filter(|e| e.0 > t0 && e.0 <= now) {
                        pts.push(pos2(x(te), y(s)));
                        pts.push(pos2(x(te), y(ns)));
                        s = ns;
                    }
                    pts.push(pos2(x(now), y(s)));
                    let stroke = ui.visuals().widgets.noninteractive.fg_stroke;
                    ui.painter().add(Shape::line(pts, stroke));
                });
            }
        });
    }
}
