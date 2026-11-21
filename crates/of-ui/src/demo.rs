//! Demo picker, live playback and instruments for the bundled ESP32-C3 firmware.

use std::time::Duration;

use egui::{
    Align2, Color32, ComboBox, Event, FontId, Pos2, Rect, RichText, Sense, Shape, Slider, Stroke, Ui, pos2, vec2,
};
use of_core::SimTime;
use of_instruments::{Delivery, SerialMonitor};

use crate::SerialMonitorPanel;
use crate::adc_pwm;
use crate::blink;
use crate::button;
use crate::kind::DemoKind;
use crate::live::{self, Command, LiveSession, Snapshot};
use crate::meters::{MeterKind, PlacedMeter, ProbeRef, place_ammeter, read_meter};
use crate::meters_ui::{MeterView, SchematicHit};
use of_components::BounceSettings;

use crate::supply::{self, SupplySettings};
use crate::theme::{LED_RED, TEAL, panel};
use crate::trace::{self, DEFAULT_TIME_DIV, TIME_DIVS};

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
    bounce: BounceSettings,
    speed: f64,
    supply: SupplySettings,
    generation: u64,
    arm: Option<MeterKind>,
    pending_a: Option<ProbeRef>,
    meters: Vec<PlacedMeter>,
    next_meter: u64,
    meter_message: Option<String>,
    time_div: f64,
    scope_hold: bool,
    held_scope: Option<HeldScope>,
    show_startup_animation: bool,
}

struct HeldScope {
    time: f64,
    trace: Vec<(f64, f64)>,
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
            bounce: BounceSettings::default(),
            speed: 1.0,
            supply: SupplySettings::default(),
            generation: 0,
            arm: None,
            pending_a: None,
            meters: Vec::new(),
            next_meter: 0,
            meter_message: None,
            time_div: DEFAULT_TIME_DIV,
            scope_hold: false,
            held_scope: None,
            show_startup_animation: crate::prefs::load().show_startup_animation,
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
        self.arm = None;
        self.pending_a = None;
        self.meters.clear();
        self.meter_message = None;
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

    fn on_probe(&mut self, probe: ProbeRef, readout: &crate::meters::CircuitReadout) {
        let Some(kind) = self.arm else {
            return;
        };
        match self.pending_a.take() {
            None => {
                self.pending_a = Some(probe);
                self.meter_message = None;
            }
            Some(a) => {
                if kind == MeterKind::Amp
                    && let Err(e) = place_ammeter(readout, &a, &probe)
                {
                    self.meter_message = Some(e.message().to_owned());
                    return;
                }
                self.next_meter = self.next_meter.saturating_add(1);
                self.meters.push(PlacedMeter {
                    id: self.next_meter,
                    kind,
                    a,
                    b: probe,
                });
                self.arm = None;
                self.meter_message = None;
            }
        }
    }

    fn meter_bar(&mut self, ui: &mut Ui, snap: &Snapshot) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Meters").strong());
            let volt = self.arm == Some(MeterKind::Volt);
            if ui.selectable_label(volt, "Voltmeter").clicked() {
                self.arm = if volt { None } else { Some(MeterKind::Volt) };
                self.pending_a = None;
                self.meter_message = None;
            }
            let amp = self.arm == Some(MeterKind::Amp);
            if ui.selectable_label(amp, "Ammeter").clicked() {
                self.arm = if amp { None } else { Some(MeterKind::Amp) };
                self.pending_a = None;
                self.meter_message = None;
            }
            if self.arm.is_some() {
                let step = if self.pending_a.is_none() {
                    "click point A, then point B"
                } else {
                    "click point B"
                };
                ui.weak(step);
            }
        });
        if let Some(msg) = &self.meter_message {
            ui.colored_label(LED_RED, msg);
        }
        let mut remove = None;
        for meter in &self.meters {
            ui.horizontal(|ui| {
                let reading = read_meter(meter, &snap.readout).display();
                ui.monospace(format!(
                    "{}  {} to {}  {reading}",
                    meter.kind.label(),
                    meter.a.label,
                    meter.b.label
                ));
                if ui.small_button("Remove").clicked() {
                    remove = Some(meter.id);
                }
            });
        }
        if let Some(id) = remove {
            self.meters.retain(|m| m.id != id);
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
            self.bounce = snap.bounce;
        }
        self.sync_serial(&snap);

        egui::ScrollArea::vertical().show(ui, |ui| {
            ui.add_space(8.0);
            ui.horizontal_wrapped(|ui| {
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
                ui.add_space(12.0);
                let mut show = self.show_startup_animation;
                if ui.checkbox(&mut show, "Show startup animation").changed() {
                    self.show_startup_animation = show;
                    let prefs = crate::prefs::AppPrefs {
                        show_startup_animation: show,
                    };
                    if let Err(err) = crate::prefs::save(&prefs) {
                        eprintln!("Ohmforge preferences: {err}");
                    }
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
                ui.weak(format!("{} requested", trace::format_multiplier(snap.speed)));
                let rt = format!("{} realtime", trace::format_multiplier(snap.rt_factor));
                if snap.lagging {
                    ui.colored_label(LED_RED, format!("{rt} (catching up)"));
                } else {
                    ui.weak(rt);
                }
            });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let mut speed = self.speed;
                let slider = Slider::new(&mut speed, live::MIN_SPEED..=live::MAX_SPEED)
                    .logarithmic(true)
                    .text("Speed")
                    .custom_formatter(|n, _| trace::format_multiplier(n));
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

            if self.kind.has_button() {
                ui.add_space(6.0);
                ui.horizontal(|ui| {
                    let mut on = self.bounce.enabled;
                    if ui.checkbox(&mut on, "SW1 bounce").changed() {
                        self.bounce.enabled = on;
                        self.session.send(Command::SetBounce(self.bounce));
                    }
                    let mut seed = self.bounce.seed;
                    if ui
                        .add(egui::DragValue::new(&mut seed).prefix("seed ").speed(1.0))
                        .changed()
                    {
                        self.bounce.seed = seed;
                        self.session.send(Command::SetBounce(self.bounce));
                    }
                    ui.weak("seed 0 is the firmware bounce test train");
                });
            }

            ui.add_space(8.0);
            self.meter_bar(ui, &snap);

            ui.add_space(8.0);
            readouts(ui, &snap);

            ui.add_space(14.0);
            if snap.booting {
                ui.label("Booting the ESP32-C3 from the fetched ROM and bootloader…");
            } else if let Some(error) = &snap.error {
                ui.colored_label(LED_RED, error);
            } else {
                let hit = {
                    let view = MeterView {
                        arm: self.arm,
                        pending_a: self.pending_a.as_ref().map(|p| p.id.as_str()),
                        meters: &self.meters,
                        readout: &snap.readout,
                    };
                    instruments(
                        ui,
                        &snap,
                        self.kind,
                        &view,
                        &mut self.time_div,
                        &mut self.scope_hold,
                        &mut self.held_scope,
                        &self.session,
                    )
                };
                if self.kind.has_button() && hit.button != self.button {
                    self.button = hit.button;
                    self.session.send(Command::SetButton(hit.button));
                }
                if let Some(probe) = hit.probe {
                    self.on_probe(probe, &snap.readout);
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

fn schematic(
    ui: &mut Ui,
    kind: DemoKind,
    current: f64,
    glow: f32,
    pos: Option<f64>,
    snap: &Snapshot,
    meters: &MeterView<'_>,
) -> SchematicHit {
    match kind {
        DemoKind::Blink => blink::show_schematic(ui, current, glow, snap.supply_status, meters),
        DemoKind::Dimmer | DemoKind::Fade => {
            adc_pwm::show_schematic(ui, current, glow, pos, snap.supply_status, meters)
        }
        DemoKind::ButtonPoll | DemoKind::ButtonIrq => {
            button::show_schematic(ui, current, glow, snap.button, snap.supply_status, meters)
        }
    }
}

/// Widget rectangles from [`scope_header`], used to test that labels do not overlap.
#[derive(Clone, Copy, Debug)]
#[cfg_attr(not(test), allow(dead_code))]
struct ScopeHeaderRects {
    title: Rect,
    wiper: Option<Rect>,
    time_div: Rect,
    hold: Rect,
}

/// Scope caption, optional secondary readout, and time/div controls.
///
/// Controls are allocated from the right first so the title can only use the
/// leftover width (truncated if needed). Extra readouts such as the pot wiper
/// sit on a second row so they cannot collide with `time/div` when the panel
/// is one column of a 980x720 window.
fn scope_header(
    ui: &mut Ui,
    title: &str,
    wiper_v: Option<f64>,
    time_div: &mut f64,
    hold: &mut bool,
) -> ScopeHeaderRects {
    let mut title_rect = Rect::NOTHING;
    let mut time_div_rect = Rect::NOTHING;
    let mut hold_rect = Rect::NOTHING;
    ui.horizontal(|ui| {
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            let hold_resp = ui
                .selectable_label(*hold, "Hold")
                .on_hover_text("Freeze the scope view for inspection");
            if hold_resp.clicked() {
                *hold = !*hold;
            }
            hold_rect = hold_resp.rect;
            let mut div = *time_div;
            ComboBox::from_id_salt("scope_time_div")
                .selected_text(trace::format_time_div(div))
                .show_ui(ui, |ui| {
                    for &d in TIME_DIVS {
                        ui.selectable_value(&mut div, d, trace::format_time_div(d));
                    }
                });
            *time_div = div;
            time_div_rect = ui.weak("time/div").rect;
            ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                ui.set_clip_rect(ui.max_rect());
                title_rect = ui.add(egui::Label::new(RichText::new(title).strong()).truncate()).rect;
            });
        });
    });
    let wiper = wiper_v.map(|v| {
        let mut rect = Rect::NOTHING;
        ui.horizontal(|ui| {
            rect = ui.monospace(format!("wiper {v:.2} V")).rect;
        });
        rect
    });
    ScopeHeaderRects {
        title: title_rect,
        wiper,
        time_div: time_div_rect,
        hold: hold_rect,
    }
}

fn apply_time_div(session: &LiveSession, time_div: &mut f64, new: f64) {
    let new = trace::snap_time_div(new);
    if (new - *time_div).abs() < 1e-15 {
        return;
    }
    *time_div = new;
    session.send(Command::SetScopeWindow(trace::visible_window(new)));
}

fn instruments(
    ui: &mut Ui,
    snap: &Snapshot,
    kind: DemoKind,
    meters: &MeterView<'_>,
    time_div: &mut f64,
    scope_hold: &mut bool,
    held_scope: &mut Option<HeldScope>,
    session: &LiveSession,
) -> SchematicHit {
    let current = snap.led_ma;
    let glow = brightness(current);
    let pos = kind.has_pot().then_some(snap.pot);
    let mut hit = SchematicHit {
        button: snap.button,
        probe: None,
    };
    if ui.available_width() >= 820.0 {
        ui.columns(2, |columns| {
            hit = schematic(&mut columns[0], kind, current, glow, pos, snap, meters);
            scope_panel(&mut columns[1], snap, kind, time_div, scope_hold, held_scope, session);
        });
    } else {
        hit = schematic(ui, kind, current, glow, pos, snap, meters);
        ui.add_space(12.0);
        scope_panel(ui, snap, kind, time_div, scope_hold, held_scope, session);
    }
    hit
}

fn scope_panel(
    ui: &mut Ui,
    snap: &Snapshot,
    kind: DemoKind,
    time_div: &mut f64,
    scope_hold: &mut bool,
    held_scope: &mut Option<HeldScope>,
    session: &LiveSession,
) {
    panel().show(ui, |ui| {
        let title = if matches!(kind, DemoKind::Dimmer | DemoKind::Fade) {
            "LED current (averaged)"
        } else {
            "LED current"
        };
        let mut hold = *scope_hold;
        let mut div = *time_div;
        scope_header(ui, title, snap.wiper_v, &mut div, &mut hold);
        if hold != *scope_hold {
            *scope_hold = hold;
            if hold {
                *held_scope = Some(HeldScope {
                    time: snap.time,
                    trace: snap.led_trace.clone(),
                });
            } else {
                *held_scope = None;
            }
        }
        if (div - *time_div).abs() > 1e-15 {
            apply_time_div(session, time_div, div);
        }

        let held = held_scope.as_ref();
        let time = held.map(|h| h.time).unwrap_or(snap.time);
        let trace_src: &[(f64, f64)] = held.map(|h| h.trace.as_slice()).unwrap_or(snap.led_trace.as_slice());
        let window = trace::visible_window(*time_div);
        let start = (time - window).max(0.0);
        let end = start + window;
        let (min_y, max_y, y_step) = if matches!(kind, DemoKind::Blink) {
            (-4.0, 8.0, 2.0)
        } else {
            (0.0, 6.0, 1.0)
        };
        let zoom = scope(
            ui,
            ScopePlot {
                time,
                start,
                end,
                min_y,
                max_y,
                y_step,
                y_unit: "mA",
                x_caption: "Simulation time",
            },
            trace_src.iter().copied(),
        );
        if zoom != 0 {
            apply_time_div(session, time_div, trace::step_time_div(*time_div, zoom));
        }
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

fn scope(ui: &mut Ui, cfg: ScopePlot, trace: impl Iterator<Item = (f64, f64)>) -> i32 {
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
    let mut axis_unit = "ms";
    for i in 0..=x_ticks {
        let t = start + span * f64::from(i) / f64::from(x_ticks);
        let x = x(t);
        let (label, unit) = trace::format_axis_time(t, span);
        axis_unit = unit;
        painter.vline(x, plot.y_range(), grid);
        painter.text(
            pos2(x, plot.bottom() + 10.0),
            Align2::CENTER_TOP,
            label,
            font.clone(),
            muted,
        );
    }
    painter.text(
        pos2(plot.center().x, response.rect.bottom()),
        Align2::CENTER_BOTTOM,
        format!("{x_caption} ({axis_unit})"),
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

    let mut zoom = 0;
    if response.hovered() {
        ui.input(|i| {
            for ev in &i.events {
                if let Event::MouseWheel { delta, .. } = ev {
                    if delta.y > 0.0 {
                        zoom = -1;
                    } else if delta.y < 0.0 {
                        zoom = 1;
                    }
                }
            }
        });
        // Stop the page ScrollArea from moving while zooming the plot.
        ui.input_mut(|i| {
            i.smooth_scroll_delta.y = 0.0;
        });
    }
    zoom
}

#[cfg(test)]
mod header_layout_tests {
    use super::*;
    use egui::Context;

    fn layout(width: f32, title: &str, wiper: Option<f64>) -> ScopeHeaderRects {
        let ctx = Context::default();
        let mut rects = ScopeHeaderRects {
            title: Rect::NOTHING,
            wiper: None,
            time_div: Rect::NOTHING,
            hold: Rect::NOTHING,
        };
        let mut time_div = DEFAULT_TIME_DIV;
        let mut hold = false;
        let raw = egui::RawInput {
            screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 280.0))),
            ..Default::default()
        };
        let mut output = ctx.run_ui(raw, |ui| {
            egui::CentralPanel::default().show(ui, |ui| {
                crate::theme::panel().show(ui, |ui| {
                    rects = scope_header(ui, title, wiper, &mut time_div, &mut hold);
                });
            });
        });
        output.textures_delta.clear();
        rects
    }

    fn assert_no_overlap(a: Rect, b: Rect, label: &str) {
        let hit = a.intersect(b);
        assert!(
            hit.width() <= 0.5 || hit.height() <= 0.5,
            "{label}: {a:?} overlaps {b:?} (intersection {hit:?})"
        );
    }

    #[test]
    fn dimmer_header_does_not_overlap_at_two_column_980() {
        // 980-wide window, 20 px margins, two instrument columns, plus panel padding.
        for width in [470.0, 420.0, 360.0, 300.0] {
            let r = layout(width, "LED current (averaged)", Some(1.65));
            assert!(r.time_div.is_positive(), "time/div missing at width {width}");
            let wiper = r.wiper.expect("wiper missing at width {width}");
            assert!(wiper.is_positive(), "wiper empty at width {width}");
            assert_no_overlap(wiper, r.time_div, &format!("wiper vs time/div at {width}"));
            assert_no_overlap(r.title, r.time_div, &format!("title vs time/div at {width}"));
            assert_no_overlap(r.title, r.hold, &format!("title vs Hold at {width}"));
            assert_no_overlap(wiper, r.hold, &format!("wiper vs Hold at {width}"));
        }
    }

    #[test]
    fn blink_and_button_headers_do_not_overlap() {
        for title in ["LED current", "LED current (averaged)"] {
            let r = layout(470.0, title, None);
            assert!(r.time_div.is_positive(), "{title}: time/div missing");
            assert!(r.wiper.is_none());
            assert_no_overlap(r.title, r.time_div, title);
            assert_no_overlap(r.title, r.hold, title);
        }
    }
}
