//! Demo picker, live playback and instruments for the bundled ESP32-C3 firmware.

use std::time::Duration;

use egui::{Align2, Color32, ComboBox, FontId, Pos2, Rect, RichText, Sense, Shape, Slider, Stroke, Ui, pos2, vec2};
use of_core::SimTime;
use of_instruments::{Delivery, SerialMonitor};

use crate::SerialMonitorPanel;
use crate::adc_pwm;
use crate::blink;
use crate::button;
use crate::kind::DemoKind;
use crate::live::{Command, LiveSession, Snapshot};
use crate::supply::{self, SupplySettings};
use crate::theme::{LED_RED, TEAL, panel};
use crate::trace;

/// Current that counts as a fully-on LED for the on-screen glow (display only).
const LED_FULL_MA: f64 = 4.0;

/// Playback controls and live instruments for the bundled firmware demos.
pub struct OhmforgeDemo {
    kind: DemoKind,
    session: LiveSession,
    monitor: SerialMonitor,
    serial_panel: SerialMonitorPanel,
    uart_cursor: usize,
    pot: f64,
    button: bool,
    speed: f64,
    supply: SupplySettings,
    generation: u64,
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
            session: LiveSession::start(DemoKind::Blink),
            monitor,
            serial_panel: SerialMonitorPanel::default(),
            uart_cursor: 0,
            pot: 0.5,
            button: false,
            speed: 1.0,
            supply: SupplySettings::default(),
            generation: 0,
        }
    }

    fn reset_monitor(&mut self) {
        self.monitor.clear();
        self.monitor.set_delivery(Delivery::Attached);
        self.uart_cursor = 0;
        self.serial_panel = SerialMonitorPanel::default();
    }

    fn restart(&mut self) {
        self.reset_monitor();
        self.session.send(Command::Reset);
    }

    fn select(&mut self, kind: DemoKind) {
        if kind == self.kind {
            return;
        }
        self.kind = kind;
        if !kind.has_pot() {
            self.pot = 0.5;
        }
        self.button = false;
        self.reset_monitor();
        self.session.send(Command::SetKind(kind));
    }

    fn sync_serial(&mut self, snap: &Snapshot) {
        let pending = self.monitor.drain_tx_queue();
        if !pending.is_empty() {
            self.session.send(Command::PushUart(pending));
        }
        self.monitor.set_baud(snap.uart_baud);
        while self.uart_cursor < snap.uart.len() {
            let b = snap.uart[self.uart_cursor];
            self.monitor.push_rx(b.at, &[b.byte]);
            self.uart_cursor += 1;
        }
    }

    /// Draw the demo. The kernel runs on the worker thread.
    pub fn ui(&mut self, ui: &mut Ui) {
        let snap = self.session.snapshot();
        if snap.generation != self.generation {
            self.generation = snap.generation;
            self.reset_monitor();
        }
        if snap.kind == self.kind && !snap.booting {
            // Keep local widgets in sync after a Reset/rebuild.
            self.supply = snap.supply;
        }
        self.sync_serial(&snap);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.heading(RichText::new(self.kind.title()).size(30.0).strong());
                ui.add_space(12.0);
                let mut kind = self.kind;
                ComboBox::from_id_salt("demo_kind")
                    .selected_text(kind.label())
                    .show_ui(ui, |ui| {
                        for k in [
                            DemoKind::Blink,
                            DemoKind::Dimmer,
                            DemoKind::Fade,
                            DemoKind::ButtonPoll,
                            DemoKind::ButtonIrq,
                        ] {
                            ui.selectable_value(&mut kind, k, k.label());
                        }
                    });
                if kind != self.kind {
                    self.select(kind);
                }
            });
            ui.add_space(4.0);
            ui.weak(self.kind.hint());
            ui.add_space(12.0);

            ui.horizontal(|ui| {
                let action = if snap.booting {
                    "Booting"
                } else if snap.playing {
                    "Pause"
                } else {
                    "Play"
                };
                let play = egui::Button::new(RichText::new(action).strong().color(Color32::from_rgb(12, 35, 31)))
                    .fill(TEAL)
                    .min_size(vec2(86.0, 32.0));
                let can_play = snap.error.is_none() && !snap.booting;
                if ui.add_enabled(can_play, play).clicked() {
                    self.session.send(Command::SetPlaying(!snap.playing));
                }
                if ui.add(egui::Button::new("Reset").min_size(vec2(64.0, 32.0))).clicked() {
                    self.restart();
                }
                ui.monospace(format!("{:.3} ms", snap.time * 1e3));
                ui.weak(format!("{:.2}× requested", snap.speed));
                let rt = format!("{:.2}× realtime", snap.rt_factor);
                if snap.lagging {
                    ui.colored_label(LED_RED, format!("{rt} (catching up)"));
                } else {
                    ui.weak(rt);
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let mut speed = self.speed;
                let slider = Slider::new(&mut speed, 0.001..=10.0)
                    .logarithmic(true)
                    .text("Speed")
                    .custom_formatter(|n, _| format!("{n:.3}×"));
                if ui.add(slider).changed() {
                    self.speed = speed;
                    self.session.send(Command::SetSpeed(speed));
                }
            });

            if self.kind.has_pot() {
                ui.add_space(8.0);
                let mut pot = self.pot;
                let slider = Slider::new(&mut pot, 0.0..=1.0)
                    .text("Potentiometer")
                    .custom_formatter(|n, _| format!("{:.0}%", n * 100.0))
                    .custom_parser(|s| s.trim().trim_end_matches('%').parse::<f64>().ok().map(|v| v / 100.0));
                if ui.add(slider).changed() {
                    self.pot = pot;
                    self.session.send(Command::SetPot(pot));
                }
            }

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                let mut on = self.supply.on;
                if ui.checkbox(&mut on, "Supply on").changed() {
                    self.supply.on = on;
                    self.session.send(Command::SetSupply(self.supply));
                }
                let mut v = self.supply.volts;
                if ui
                    .add(Slider::new(&mut v, 0.0..=5.0).text("Vset").suffix(" V").min_decimals(2))
                    .changed()
                {
                    self.supply.volts = v;
                    self.session.send(Command::SetSupply(self.supply));
                }
                let mut rs = self.supply.series_r;
                if ui
                    .add(
                        Slider::new(&mut rs, 0.01..=10.0)
                            .logarithmic(true)
                            .text("Rs")
                            .suffix(" Ω"),
                    )
                    .changed()
                {
                    self.supply.series_r = rs;
                    self.session.send(Command::SetSupply(self.supply));
                }
                let mut lim = self.supply.current_limit * 1e3;
                if ui
                    .add(
                        Slider::new(&mut lim, 0.1..=1000.0)
                            .logarithmic(true)
                            .text("Ilim")
                            .suffix(" mA"),
                    )
                    .changed()
                {
                    self.supply.current_limit = lim / 1e3;
                    self.session.send(Command::SetSupply(self.supply));
                }
            });

            let status = snap.supply_status;
            ui.horizontal(|ui| {
                ui.monospace(format!(
                    "PSU  {:.2} V  {:.2} mA  {}",
                    status.volts,
                    status.amps * 1e3,
                    supply::mode_label(status.mode)
                ));
            });
            if let Some(warn) = supply::rating_warning(&status) {
                ui.colored_label(LED_RED, warn);
            }

            ui.add_space(8.0);
            readouts(ui, &snap);

            ui.add_space(14.0);
            if snap.booting {
                ui.label("Booting the ESP32-C3 from the fetched ROM and bootloader…");
            } else if let Some(error) = &snap.error {
                ui.colored_label(LED_RED, error);
            } else {
                let pressed = instruments(ui, &snap, self.kind);
                if self.kind.has_button() && pressed != self.button {
                    self.button = pressed;
                    self.session.send(Command::SetButton(pressed));
                }
                ui.add_space(12.0);
                let now = SimTime::from_secs_f64(snap.time);
                self.serial_panel.show(ui, &mut self.monitor, "UART0", now);
            }
        });

        if snap.playing || snap.booting {
            ui.ctx().request_repaint_after(Duration::from_millis(16));
        }
    }
}

fn brightness(ma: f64) -> f32 {
    (ma / LED_FULL_MA).clamp(0.0, 1.0) as f32
}

fn readouts(ui: &mut Ui, snap: &Snapshot) {
    ui.horizontal_wrapped(|ui| {
        ui.monospace(format!("LED {:.2} mA", snap.led_ma));
        if let Some(v) = snap.wiper_v {
            ui.separator();
            ui.monospace(format!("wiper {v:.2} V"));
        }
        if let Some((mv, duty)) = snap.adc {
            ui.separator();
            ui.monospace(format!("ADC {mv} mV  duty {duty}"));
        }
        if let (Some(d), Some(f)) = (snap.pwm_duty, snap.pwm_freq) {
            ui.separator();
            ui.monospace(format!("PWM {d:.1}%  {f:.0} Hz"));
        }
        if let Some(v) = snap.button_v {
            ui.separator();
            ui.monospace(format!("GPIO9 {v:.2} V"));
        }
    });
}

fn instruments(ui: &mut Ui, snap: &Snapshot, kind: DemoKind) -> bool {
    let current = snap.led_ma;
    let glow = brightness(current);
    let pos = kind.has_pot().then_some(snap.pot);
    let mut pressed = snap.button;
    if ui.available_width() >= 820.0 {
        ui.columns(2, |columns| {
            pressed = schematic(&mut columns[0], kind, current, glow, pos, snap);
            scope_panel(&mut columns[1], snap, kind);
        });
    } else {
        pressed = schematic(ui, kind, current, glow, pos, snap);
        ui.add_space(12.0);
        scope_panel(ui, snap, kind);
    }
    pressed
}

fn schematic(ui: &mut Ui, kind: DemoKind, current: f64, glow: f32, pos: Option<f64>, snap: &Snapshot) -> bool {
    match kind {
        DemoKind::Blink => {
            blink::show_schematic(ui, current, glow, snap.supply_status);
            false
        }
        DemoKind::Dimmer | DemoKind::Fade => {
            adc_pwm::show_schematic(ui, current, glow, pos, snap.supply_status);
            false
        }
        DemoKind::ButtonPoll | DemoKind::ButtonIrq => {
            button::show_schematic(ui, current, glow, snap.button, snap.supply_status)
        }
    }
}

fn scope_panel(ui: &mut Ui, snap: &Snapshot, kind: DemoKind) {
    panel().show(ui, |ui| {
        ui.horizontal(|ui| {
            let title = if matches!(kind, DemoKind::Dimmer | DemoKind::Fade) {
                "LED current (averaged)"
            } else {
                "LED current"
            };
            ui.label(RichText::new(title).strong());
            if let Some(v) = snap.wiper_v {
                ui.separator();
                ui.monospace(format!("wiper {v:.2} V"));
            }
        });
        let time = snap.time;
        let window = 0.12;
        let start = (time - window).max(0.0);
        let end = start + window;
        let (min_y, max_y, y_step) = if matches!(kind, DemoKind::Blink) {
            (-4.0, 8.0, 2.0)
        } else {
            (0.0, 6.0, 1.0)
        };
        scope(
            ui,
            ScopePlot {
                time,
                start,
                end,
                min_y,
                max_y,
                y_step,
                y_unit: "mA",
                x_caption: "Simulation time (ms)",
            },
            snap.led_trace.iter().copied(),
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

// Referenced so unused-import lint stays quiet if TRACE_WINDOW is only used by the worker.
#[allow(dead_code)]
fn _trace_window() -> f64 {
    trace::TRACE_WINDOW
}
