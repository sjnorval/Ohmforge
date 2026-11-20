mod schematic;
mod simulation;

use std::time::Duration;

use egui::{Align2, Color32, FontId, Frame, Margin, Pos2, Rect, RichText, Sense, Shape, Stroke, Ui, pos2, vec2};
use of_core::SimTime;
use of_instruments::{Delivery, SerialMonitor};

use crate::SerialMonitorPanel;
use simulation::{Blink, END};

const PLAYBACK_RATE: f64 = 1.0 / 2_000.0;
const TEAL: Color32 = Color32::from_rgb(99, 222, 199);
const LED_RED: Color32 = Color32::from_rgb(255, 93, 97);
const PANEL: Color32 = Color32::from_rgb(22, 26, 32);

/// Playback controls and live instruments for the bundled ESP32-C3 blink firmware.
pub struct BlinkDemo {
    sim: Result<Blink, Box<dyn std::error::Error>>,
    playing: Option<(f64, f64)>,
    monitor: SerialMonitor,
    serial_panel: SerialMonitorPanel,
    uart_cursor: usize,
}

impl Default for BlinkDemo {
    fn default() -> Self {
        Self::new()
    }
}

impl BlinkDemo {
    /// Load the bundled firmware and initialise the LED circuit.
    pub fn new() -> Self {
        let mut monitor = SerialMonitor::new();
        monitor.set_delivery(Delivery::Attached);
        Self {
            sim: Blink::new(),
            playing: None,
            monitor,
            serial_panel: SerialMonitorPanel::default(),
            uart_cursor: 0,
        }
    }

    fn restart(&mut self) {
        self.playing = None;
        self.sim = Blink::new();
        self.monitor.clear();
        self.monitor.set_delivery(Delivery::Attached);
        self.uart_cursor = 0;
        self.serial_panel = SerialMonitorPanel::default();
    }

    fn sync_serial(&mut self) {
        let pending = self.monitor.drain_tx_queue();
        let Ok(sim) = &mut self.sim else {
            return;
        };
        sim.push_uart_rx(&pending);
        self.monitor.set_baud(sim.uart_baud());
        let bytes: Vec<_> = sim.uart_bytes().to_vec();
        while self.uart_cursor < bytes.len() {
            let b = bytes[self.uart_cursor];
            self.monitor.push_rx(b.at, &[b.byte]);
            self.uart_cursor += 1;
        }
    }

    fn tick(&mut self, now: f64) {
        let (Some((started, from)), Ok(sim)) = (self.playing, &mut self.sim) else {
            return;
        };
        let target = from + (now - started) * PLAYBACK_RATE;
        if let Err(error) = sim.advance(target) {
            self.sim = Err(error);
            self.playing = None;
        } else if sim.finished() {
            self.playing = None;
        }
    }

    /// Draw the demo and advance virtual time during playback.
    pub fn ui(&mut self, ui: &mut Ui) {
        let now = ui.input(|input| input.time);
        self.tick(now);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            ui.heading(RichText::new("ESP32-C3 LED blink").size(30.0).strong());
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let action = match (&self.sim, self.playing) {
                    (_, Some(_)) => "Pause",
                    (Ok(sim), _) if sim.finished() => "Replay",
                    _ => "Play",
                };
                let play = egui::Button::new(RichText::new(action).strong().color(Color32::from_rgb(12, 35, 31)))
                    .fill(TEAL)
                    .min_size(vec2(86.0, 32.0));
                if ui.add_enabled(self.sim.is_ok(), play).clicked() && self.playing.take().is_none() {
                    if self.sim.as_ref().is_ok_and(Blink::finished) {
                        self.restart();
                    }
                    if let Ok(sim) = &self.sim {
                        self.playing = Some((now, sim.time()));
                    }
                }
                if ui.add(egui::Button::new("Reset").min_size(vec2(64.0, 32.0))).clicked() {
                    self.restart();
                }
                if let Ok(sim) = &self.sim {
                    ui.monospace(format!("{:.3} / {:.3} ms", sim.time() * 1e3, END.as_secs_f64() * 1e3));
                    ui.weak("2,000× slower");
                }
            });
            ui.add_space(14.0);

            self.sync_serial();
            match &mut self.sim {
                Ok(sim) => {
                    instruments(ui, sim);
                    ui.add_space(12.0);
                    let now = SimTime::from_secs_f64(sim.time());
                    self.serial_panel.show(ui, &mut self.monitor, "UART0", now);
                }
                Err(error) => {
                    ui.colored_label(LED_RED, format!("Simulation error: {error}"));
                }
            }
        });
        if self.playing.is_some() {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn panel() -> Frame {
    Frame::new().fill(PANEL).corner_radius(8).inner_margin(Margin::same(14))
}

fn instruments(ui: &mut Ui, sim: &mut Blink) {
    let current = match sim.current_ma() {
        Ok(current) => current,
        Err(error) => {
            ui.colored_label(LED_RED, format!("Simulation error: {error}"));
            return;
        }
    };
    if ui.available_width() >= 820.0 {
        ui.columns(2, |columns| {
            schematic::show(&mut columns[0], current);
            scope_panel(&mut columns[1], sim);
        });
    } else {
        schematic::show(ui, current);
        ui.add_space(12.0);
        scope_panel(ui, sim);
    }
}

fn scope_panel(ui: &mut Ui, sim: &mut Blink) {
    panel().show(ui, |ui| {
        ui.label(RichText::new("LED current").strong());
        let time = sim.time();
        match sim.trace() {
            Ok(trace) => scope(ui, time, trace),
            Err(error) => {
                ui.colored_label(LED_RED, format!("Simulation error: {error}"));
            }
        }
    });
}

fn scope(ui: &mut Ui, time: f64, trace: impl Iterator<Item = (f64, f64)>) {
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), 242.0), Sense::hover());
    let plot = Rect::from_min_max(
        response.rect.min + vec2(45.0, 12.0),
        response.rect.max - vec2(12.0, 42.0),
    );
    let end = END.as_secs_f64();
    let x = |seconds: f64| plot.left() + plot.width() * (seconds / end) as f32;
    // Include the capacitive current spikes in addition to the steady on-current.
    const MIN_MA: i32 = -4;
    const MAX_MA: i32 = 8;
    let y = |ma: f64| plot.bottom() - plot.height() * ((ma - f64::from(MIN_MA)) / f64::from(MAX_MA - MIN_MA)) as f32;
    let grid = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let muted = ui.visuals().weak_text_color();
    let font = FontId::monospace(12.0);

    for ma in (MIN_MA..=MAX_MA).step_by(2) {
        let y = y(f64::from(ma));
        painter.hline(plot.x_range(), y, grid);
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{ma} mA"),
            font.clone(),
            muted,
        );
    }
    for tick in 0..=5 {
        let ms = f64::from(tick) / 2.0;
        let x = x(ms * 1e-3);
        painter.vline(x, plot.y_range(), grid);
        painter.text(
            pos2(x, plot.bottom() + 10.0),
            Align2::CENTER_TOP,
            format!("{ms:.1}"),
            font.clone(),
            muted,
        );
    }
    painter.text(
        pos2(plot.center().x, response.rect.bottom()),
        Align2::CENTER_BOTTOM,
        "Simulation time (ms)",
        font,
        muted,
    );

    let points: Vec<Pos2> = trace.map(|(t, ma)| pos2(x(t), y(ma))).collect();
    if let Some(&last) = points.last() {
        let scope = painter.with_clip_rect(plot.expand(2.0));
        scope.vline(x(time), plot.y_range(), Stroke::new(1.0, muted));
        if points.len() > 1 {
            scope.add(Shape::line(points, Stroke::new(2.0, TEAL)));
        }
        scope.circle_filled(last, 3.0, TEAL);
    }
}
