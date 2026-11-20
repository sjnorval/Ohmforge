//! Demo picker, playback and instruments for the bundled ESP32-C3 firmware.

use std::thread::JoinHandle;
use std::time::Duration;

use egui::{Align2, Color32, ComboBox, FontId, Pos2, Rect, RichText, Sense, Shape, Slider, Stroke, Ui, pos2, vec2};
use of_core::SimTime;
use of_instruments::{Delivery, SerialMonitor};

use crate::SerialMonitorPanel;
use crate::adc_pwm::{self, PwmKind, PwmSim};
use crate::blink::{self, Blink, END as BLINK_END};
use crate::theme::{LED_RED, TEAL, panel};

const BLINK_RATE: f64 = 1.0 / 2_000.0;
const PWM_RATE: f64 = 1.0 / 10.0;
/// Current that counts as a fully-on LED for the on-screen glow (display only).
const LED_FULL_MA: f64 = 4.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DemoKind {
    Blink,
    Dimmer,
    Fade,
}

impl DemoKind {
    fn label(self) -> &'static str {
        match self {
            Self::Blink => "LED blink",
            Self::Dimmer => "Potentiometer dimmer",
            Self::Fade => "LED fade",
        }
    }

    fn title(self) -> &'static str {
        match self {
            Self::Blink => "ESP32-C3 LED blink",
            Self::Dimmer => "ESP32-C3 potentiometer dimmer",
            Self::Fade => "ESP32-C3 LED fade",
        }
    }

    fn hint(self) -> &'static str {
        match self {
            Self::Blink => "Bundled bare-metal firmware toggles GPIO8. No ROM download required.",
            Self::Dimmer => "esp-hal reads the pot on GPIO2 (ADC) and sets a 1 kHz LEDC duty on GPIO8.",
            Self::Fade => "esp-hal fades the LED on GPIO8 up and down in hardware (LEDC, 5 kHz).",
        }
    }

    fn rate(self) -> f64 {
        match self {
            Self::Blink => BLINK_RATE,
            Self::Dimmer | Self::Fade => PWM_RATE,
        }
    }

    fn rate_label(self) -> &'static str {
        match self {
            Self::Blink => "2,000× slower",
            Self::Dimmer | Self::Fade => "10× slower",
        }
    }
}

enum Sim {
    Blink(Result<Box<Blink>, Box<dyn std::error::Error>>),
    Pwm(Result<Box<PwmSim>, Box<dyn std::error::Error>>),
    /// Waiting for the ROM boot to finish on a background thread.
    Booting,
}

/// Playback controls and live instruments for the bundled firmware demos.
pub struct OhmforgeDemo {
    kind: DemoKind,
    sim: Sim,
    playing: Option<(f64, f64)>,
    monitor: SerialMonitor,
    serial_panel: SerialMonitorPanel,
    uart_cursor: usize,
    pot: f64,
    boot: Option<JoinHandle<Result<PwmSim, String>>>,
}

impl Default for OhmforgeDemo {
    fn default() -> Self {
        Self::new()
    }
}

impl OhmforgeDemo {
    /// Start on the LED blink demo, which needs no fetched ROM.
    pub fn new() -> Self {
        let mut monitor = SerialMonitor::new();
        monitor.set_delivery(Delivery::Attached);
        Self {
            kind: DemoKind::Blink,
            sim: Sim::Blink(Blink::new().map(Box::new)),
            playing: None,
            monitor,
            serial_panel: SerialMonitorPanel::default(),
            uart_cursor: 0,
            pot: 0.5,
            boot: None,
        }
    }

    fn reset_monitor(&mut self) {
        self.monitor.clear();
        self.monitor.set_delivery(Delivery::Attached);
        self.uart_cursor = 0;
        self.serial_panel = SerialMonitorPanel::default();
    }

    fn restart(&mut self) {
        self.playing = None;
        self.reset_monitor();
        match self.kind {
            DemoKind::Blink => {
                self.boot = None;
                self.sim = Sim::Blink(Blink::new().map(Box::new));
            }
            DemoKind::Dimmer | DemoKind::Fade => self.start_pwm(),
        }
    }

    fn start_pwm(&mut self) {
        self.playing = None;
        let kind = match self.kind {
            DemoKind::Dimmer => PwmKind::Dimmer,
            DemoKind::Fade => PwmKind::Fade,
            DemoKind::Blink => return,
        };
        // A cached SoC is cheap to clone. If the ROM has not been booted yet,
        // do that off the UI thread so a "Booting…" label can paint first.
        // A cached error (missing ROM) is shown immediately.
        match adc_pwm::cached(kind) {
            Some(Ok(_)) => {
                self.boot = None;
                self.sim = Sim::Pwm(PwmSim::new(kind, self.pot).map(Box::new));
            }
            Some(Err(msg)) => {
                self.boot = None;
                self.sim = Sim::Pwm(Err(msg.clone().into()));
            }
            None => {
                let pot = self.pot;
                self.sim = Sim::Booting;
                self.boot = Some(std::thread::spawn(move || {
                    PwmSim::new(kind, pot).map_err(|e| e.to_string())
                }));
            }
        }
    }

    fn poll_boot(&mut self) {
        let Some(handle) = self.boot.as_ref() else { return };
        if !handle.is_finished() {
            return;
        }
        let handle = self.boot.take().expect("finished handle");
        self.sim = match handle.join() {
            Ok(Ok(sim)) => Sim::Pwm(Ok(Box::new(sim))),
            Ok(Err(msg)) => Sim::Pwm(Err(msg.into())),
            Err(_) => Sim::Pwm(Err("boot thread panicked".into())),
        };
    }

    fn select(&mut self, kind: DemoKind) {
        if kind == self.kind {
            return;
        }
        self.kind = kind;
        if kind != DemoKind::Dimmer {
            self.pot = 0.5;
        }
        self.restart();
    }

    fn sync_serial(&mut self) {
        let pending = self.monitor.drain_tx_queue();
        match &mut self.sim {
            Sim::Blink(Ok(sim)) => {
                sim.push_uart_rx(&pending);
                self.monitor.set_baud(sim.uart_baud());
                let bytes: Vec<_> = sim.uart_bytes().to_vec();
                while self.uart_cursor < bytes.len() {
                    let b = bytes[self.uart_cursor];
                    self.monitor.push_rx(b.at, &[b.byte]);
                    self.uart_cursor += 1;
                }
            }
            Sim::Pwm(Ok(sim)) => {
                sim.push_uart_rx(&pending);
                self.monitor.set_baud(sim.uart_baud());
                let bytes: Vec<_> = sim.uart_bytes().to_vec();
                while self.uart_cursor < bytes.len() {
                    let b = bytes[self.uart_cursor];
                    self.monitor.push_rx(b.at, &[b.byte]);
                    self.uart_cursor += 1;
                }
            }
            _ => {}
        }
    }

    fn tick(&mut self, now: f64) {
        let Some((started, from)) = self.playing else { return };
        let target = from + (now - started) * self.kind.rate();
        let result = match &mut self.sim {
            Sim::Blink(Ok(sim)) => sim.advance(target),
            Sim::Pwm(Ok(sim)) => sim.advance(target),
            _ => return,
        };
        if let Err(error) = result {
            self.sim = match self.kind {
                DemoKind::Blink => Sim::Blink(Err(error)),
                DemoKind::Dimmer | DemoKind::Fade => Sim::Pwm(Err(error)),
            };
            self.playing = None;
        } else if matches!(&self.sim, Sim::Blink(Ok(sim)) if sim.finished()) {
            self.playing = None;
        }
    }

    fn sim_ok(&self) -> bool {
        matches!(&self.sim, Sim::Blink(Ok(_)) | Sim::Pwm(Ok(_)))
    }

    fn sim_time(&self) -> Option<f64> {
        match &self.sim {
            Sim::Blink(Ok(sim)) => Some(sim.time()),
            Sim::Pwm(Ok(sim)) => Some(sim.time()),
            _ => None,
        }
    }

    fn finished_blink(&self) -> bool {
        matches!(&self.sim, Sim::Blink(Ok(sim)) if sim.finished())
    }

    /// Draw the demo and advance virtual time during playback.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.poll_boot();
        let now = ui.input(|input| input.time);
        self.tick(now);
        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new(self.kind.title()).size(30.0).strong());
                ui.add_space(12.0);
                let mut kind = self.kind;
                ComboBox::from_id_salt("demo_kind")
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut kind, DemoKind::Blink, DemoKind::Blink.label());
                        ui.selectable_value(&mut kind, DemoKind::Dimmer, DemoKind::Dimmer.label());
                        ui.selectable_value(&mut kind, DemoKind::Fade, DemoKind::Fade.label());
                    });
                if kind != self.kind {
                    self.select(kind);
                }
            });
            ui.add_space(4.0);
            ui.weak(self.kind.hint());
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let action = match (&self.sim, self.playing, self.finished_blink()) {
                    (Sim::Booting, _, _) => "Booting",
                    (_, Some(_), _) => "Pause",
                    (_, _, true) => "Replay",
                    _ => "Play",
                };
                let play = egui::Button::new(RichText::new(action).strong().color(Color32::from_rgb(12, 35, 31)))
                    .fill(TEAL)
                    .min_size(vec2(86.0, 32.0));
                let can_play = self.sim_ok();
                if ui.add_enabled(can_play, play).clicked() && self.playing.take().is_none() {
                    if self.finished_blink() {
                        self.restart();
                    }
                    if let Some(time) = self.sim_time() {
                        self.playing = Some((now, time));
                    }
                }
                if ui.add(egui::Button::new("Reset").min_size(vec2(64.0, 32.0))).clicked() {
                    self.restart();
                }
                match &self.sim {
                    Sim::Blink(Ok(sim)) => {
                        ui.monospace(format!(
                            "{:.3} / {:.3} ms",
                            sim.time() * 1e3,
                            BLINK_END.as_secs_f64() * 1e3
                        ));
                        ui.weak(self.kind.rate_label());
                    }
                    Sim::Pwm(Ok(sim)) => {
                        ui.monospace(format!("{:.3} ms", sim.time() * 1e3));
                        ui.weak(self.kind.rate_label());
                    }
                    _ => {}
                }
            });

            if self.kind == DemoKind::Dimmer {
                ui.add_space(8.0);
                let mut pot = self.pot;
                let slider = Slider::new(&mut pot, 0.0..=1.0)
                    .text("Potentiometer")
                    .custom_formatter(|n, _| format!("{:.0}%", n * 100.0))
                    .custom_parser(|s| s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0));
                if ui.add(slider).changed() {
                    self.pot = pot;
                    if let Sim::Pwm(Ok(sim)) = &mut self.sim
                        && let Err(error) = sim.set_position(pot)
                    {
                        self.sim = Sim::Pwm(Err(error));
                        self.playing = None;
                    }
                }
            }

            ui.add_space(14.0);
            self.sync_serial();
            match &mut self.sim {
                Sim::Blink(Ok(sim)) => {
                    instruments_blink(ui, sim);
                    ui.add_space(12.0);
                    let now = SimTime::from_secs_f64(sim.time());
                    self.serial_panel.show(ui, &mut self.monitor, "UART0", now);
                }
                Sim::Pwm(Ok(sim)) => {
                    instruments_pwm(ui, sim);
                    ui.add_space(12.0);
                    let now = SimTime::from_secs_f64(sim.time());
                    self.serial_panel.show(ui, &mut self.monitor, "UART0", now);
                }
                Sim::Blink(Err(error)) | Sim::Pwm(Err(error)) => {
                    ui.colored_label(LED_RED, error.to_string());
                }
                Sim::Booting => {
                    ui.label("Booting the ESP32-C3 from the fetched ROM and bootloader…");
                }
            }
        });
        if self.playing.is_some() || matches!(self.sim, Sim::Booting) {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn brightness(ma: f64) -> f32 {
    (ma / LED_FULL_MA).clamp(0.0, 1.0) as f32
}

fn instruments_blink(ui: &mut Ui, sim: &mut Blink) {
    let current = match sim.current_ma() {
        Ok(current) => current,
        Err(error) => {
            ui.colored_label(LED_RED, format!("Simulation error: {error}"));
            return;
        }
    };
    if ui.available_width() >= 820.0 {
        ui.columns(2, |columns| {
            blink::show_schematic(&mut columns[0], current, brightness(current));
            scope_panel_blink(&mut columns[1], sim);
        });
    } else {
        blink::show_schematic(ui, current, brightness(current));
        ui.add_space(12.0);
        scope_panel_blink(ui, sim);
    }
}

fn instruments_pwm(ui: &mut Ui, sim: &mut PwmSim) {
    let current = sim.average_ma();
    let pos = (sim.kind() == PwmKind::Dimmer).then_some(sim.position());
    let wiper = if pos.is_some() { sim.wiper_volts().ok() } else { None };
    if ui.available_width() >= 820.0 {
        ui.columns(2, |columns| {
            adc_pwm::show_schematic(&mut columns[0], current, brightness(current), pos);
            scope_panel_pwm(&mut columns[1], sim, wiper);
        });
    } else {
        adc_pwm::show_schematic(ui, current, brightness(current), pos);
        ui.add_space(12.0);
        scope_panel_pwm(ui, sim, wiper);
    }
}

fn scope_panel_blink(ui: &mut Ui, sim: &mut Blink) {
    panel().show(ui, |ui| {
        ui.label(RichText::new("LED current").strong());
        let time = sim.time();
        match sim.trace() {
            Ok(trace) => scope(
                ui,
                ScopePlot {
                    time,
                    start: 0.0,
                    end: BLINK_END.as_secs_f64(),
                    min_y: -4.0,
                    max_y: 8.0,
                    y_step: 2.0,
                    y_unit: "mA",
                    x_caption: "Simulation time (ms)",
                },
                trace,
            ),
            Err(error) => {
                ui.colored_label(LED_RED, format!("Simulation error: {error}"));
            }
        }
    });
}

fn scope_panel_pwm(ui: &mut Ui, sim: &mut PwmSim, wiper: Option<f64>) {
    panel().show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("LED current (averaged)").strong());
            if let Some(v) = wiper {
                ui.separator();
                ui.monospace(format!("wiper {v:.2} V"));
            }
        });
        let time = sim.time();
        let window = 0.12;
        let start = (time - window).max(0.0);
        let end = start + window;
        scope(
            ui,
            ScopePlot {
                time,
                start,
                end,
                min_y: 0.0,
                max_y: 6.0,
                y_step: 1.0,
                y_unit: "mA",
                x_caption: "Simulation time (ms)",
            },
            sim.trace().into_iter(),
        );
    });
}

struct ScopePlot {
    time: f64,
    start: f64,
    end: f64,
    min_y: f64,
    max_y: f64,
    y_step: f64,
    y_unit: &'static str,
    x_caption: &'static str,
}

fn scope(ui: &mut Ui, cfg: ScopePlot, trace: impl Iterator<Item = (f64, f64)>) {
    let ScopePlot {
        time,
        start,
        end,
        min_y,
        max_y,
        y_step,
        y_unit,
        x_caption,
    } = cfg;
    let (response, painter) = ui.allocate_painter(vec2(ui.available_width(), 242.0), Sense::hover());
    let plot = Rect::from_min_max(
        response.rect.min + vec2(45.0, 12.0),
        response.rect.max - vec2(12.0, 42.0),
    );
    let span = (end - start).max(1e-12);
    let x = |seconds: f64| plot.left() + plot.width() * ((seconds - start) / span) as f32;
    let y = |val: f64| plot.bottom() - plot.height() * ((val - min_y) / (max_y - min_y)) as f32;
    let grid = Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color);
    let muted = ui.visuals().weak_text_color();
    let font = FontId::monospace(12.0);

    let mut tick = min_y;
    while tick <= max_y + 1e-9 {
        let y = y(tick);
        painter.hline(plot.x_range(), y, grid);
        painter.text(
            pos2(plot.left() - 8.0, y),
            Align2::RIGHT_CENTER,
            format!("{tick:.0} {y_unit}"),
            font.clone(),
            muted,
        );
        tick += y_step;
    }
    let x_ticks = 5;
    for i in 0..=x_ticks {
        let t = start + span * f64::from(i) / f64::from(x_ticks);
        let x = x(t);
        painter.vline(x, plot.y_range(), grid);
        painter.text(
            pos2(x, plot.bottom() + 10.0),
            Align2::CENTER_TOP,
            format!("{:.1}", t * 1e3),
            font.clone(),
            muted,
        );
    }
    painter.text(
        pos2(plot.center().x, response.rect.bottom()),
        Align2::CENTER_BOTTOM,
        x_caption,
        font,
        muted,
    );

    let points: Vec<Pos2> = trace
        .filter(|(t, _)| *t >= start - 1e-12 && *t <= end + 1e-12)
        .map(|(t, val)| pos2(x(t), y(val)))
        .collect();
    if let Some(&last) = points.last() {
        let scope = painter.with_clip_rect(plot.expand(2.0));
        if time >= start && time <= end {
            scope.vline(x(time), plot.y_range(), Stroke::new(1.0, muted));
        }
        if points.len() > 1 {
            scope.add(Shape::line(points, Stroke::new(2.0, TEAL)));
        }
        scope.circle_filled(last, 3.0, TEAL);
    }
}
