//! Demo picker, live playback and instruments for the bundled ESP32-C3 firmware.

use std::time::Duration;

use egui::{
    Align2, Color32, ComboBox, Event, FontId, Pos2, Rect, RichText, Sense, Shape, Slider, Stroke, Ui, pos2, vec2,
};
use of_core::SimTime;
use of_instruments::{Candidate, Delivery, Multimeter, PartPins, SerialMonitor, candidates};

use crate::SerialMonitorPanel;
use crate::adc_pwm;
use crate::blink;
use crate::boards;
use crate::button;
use crate::kind::DemoKind;
use crate::live::{self, Command, LiveSession, Snapshot};
use crate::meters::{MeterKind, PlacedMeter, ProbeRef};
use of_instruments::profile;

use crate::bench::BenchProfiles;
use crate::meters_ui::{MeterView, SchematicHit, reading_text};
use of_components::BounceSettings;

use crate::console::Console;
use crate::console::log::{Level, Source};
use crate::status::{Message, StatusLog};
use crate::supply::{self, SupplySettings};
use crate::theme::{LED_RED, TEAL, panel};
use crate::trace::{self, DEFAULT_TIME_DIV, TIME_DIVS};
use of_circuit::library::RailPart;

/// Current that counts as a fully-on LED for the on-screen glow (display only).
const LED_FULL_MA: f64 = 4.0;

/// How long a change in the real-time lag must hold before the log reports it
/// (s); busy demos hover at the threshold and would log every few frames.
const LAG_LOG_HOLD_S: f64 = 2.0;

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
    /// An ammeter whose two points several parts join: the user picks one.
    choosing: Option<(ProbeRef, ProbeRef, Vec<Candidate>)>,
    next_meter: u64,
    meter_message: Option<String>,
    time_div: f64,
    scope_hold: bool,
    held_scope: Option<HeldScope>,
    show_startup_animation: bool,
    /// Real-world profiles of the meters, scope and PSU readout (ideal by default).
    bench: BenchProfiles,
    /// The parts on the bench supply's + rail that state a supply range.
    rail: Vec<RailPart>,
    /// Latched warnings and errors for the status strip.
    status: StatusLog,
    /// The `~` log console.
    console: Console,
    /// UART bytes since the last newline (the log takes whole lines).
    uart_line: Vec<u8>,
    /// App time of this frame (s), for log entries.
    now: f64,
    was_booting: bool,
    /// The real-time lag state the log last reported, and since when the
    /// snapshots have said otherwise (it must hold [`LAG_LOG_HOLD_S`]).
    was_lagging: bool,
    lag_changed_at: Option<f64>,
    /// The cached schematic fit is from another demo (reset next frame).
    fits_stale: bool,
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
    /// Start on the LED blink demo, which needs no fetched ROM. Every demo's
    /// schematic and simulation are built from its embedded circuit file
    /// (`circuits/demos`).
    pub fn new() -> Self {
        let mut monitor = SerialMonitor::new();
        monitor.set_delivery(Delivery::Attached);
        let prefs = crate::prefs::load();
        let mut demo = Self {
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
            choosing: None,
            next_meter: 0,
            meter_message: None,
            time_div: DEFAULT_TIME_DIV,
            scope_hold: false,
            held_scope: None,
            bench: BenchProfiles::default(),
            rail: crate::session_sim::rail_parts(DemoKind::Blink),
            status: StatusLog::default(),
            console: Console::new(prefs.console.settings(), prefs.console.width()),
            uart_line: Vec::new(),
            now: 0.0,
            was_booting: false,
            was_lagging: false,
            lag_changed_at: None,
            fits_stale: true,
            show_startup_animation: prefs.show_startup_animation,
        };
        demo.log_loaded();
        demo
    }

    /// Log a message at this frame's time (and the simulated time when given).
    fn log(&mut self, level: Level, source: Source, text: impl Into<String>, sim: Option<f64>) {
        self.console.push(level, source, text, self.now, sim);
    }

    /// The demo that was just loaded, and its circuit file if it fails to parse.
    fn log_loaded(&mut self) {
        let path = crate::session_sim::demo_file(self.kind).path;
        self.log(
            Level::Info,
            Source::Ui,
            format!("Loaded {} ({path})", self.kind.title()),
            None,
        );
        if let Err(e) = crate::session_sim::parsed(self.kind) {
            self.log(Level::Error, Source::Parts, format!("{path}: {e}"), None);
        }
    }

    /// Play or pause the simulation: the main Play/Pause and the console's
    /// Pause sim / Resume sim both come here, and both read the state from
    /// the same snapshot, so they stay in sync.
    fn toggle_play(&mut self, snap: &Snapshot) {
        self.session.send(Command::SetPlaying(!snap.playing));
        let text = if snap.playing { "Paused" } else { "Playing" };
        self.log(Level::Info, Source::Ui, text, Some(snap.time));
    }

    /// Save the console's settings without touching the other preferences.
    fn save_console_prefs(&mut self) {
        let mut prefs = crate::prefs::load();
        prefs.console = crate::prefs::ConsolePrefs::from_settings(self.console.log.settings(), self.console.width());
        if let Err(err) = crate::prefs::save(&prefs) {
            self.log(
                Level::Error,
                Source::Ui,
                format!("Could not save preferences: {err}"),
                None,
            );
        }
    }

    fn reset_monitor(&mut self) {
        self.monitor.clear();
        self.monitor.set_delivery(Delivery::Attached);
        self.uart_cursor = 0;
        self.uart_line.clear();
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
        self.rail = crate::session_sim::rail_parts(kind);
        self.status.clear();
        self.log_loaded();
        self.fits_stale = true;
        if !kind.has_pot() {
            self.pot = 0.5;
        }
        self.button = false;
        self.arm = None;
        self.pending_a = None;
        self.meters.clear();
        self.choosing = None;
        self.meter_message = None;
        self.sync_meters();
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
            // Whole lines go to the log (a runaway line is cut at 256 bytes).
            if b.byte == b'\n' || self.uart_line.len() >= 256 {
                let line = String::from_utf8_lossy(&self.uart_line).trim_end().to_owned();
                self.uart_line.clear();
                if !line.is_empty() {
                    self.log(Level::Info, Source::Firmware, line, Some(b.at.as_secs_f64()));
                }
            }
            if b.byte != b'\n' && b.byte != b'\r' {
                self.uart_line.push(b.byte);
            }
        }
    }

    /// Tell the worker which meters to sample.
    fn sync_meters(&self) {
        let meters = self.meters.iter().map(|m| (m.id, m.meter.clone())).collect();
        self.session.send(Command::SetMeters(meters));
    }

    fn place(&mut self, kind: MeterKind, a: ProbeRef, b: ProbeRef, meter: Multimeter) {
        self.next_meter = self.next_meter.saturating_add(1);
        self.meters.push(PlacedMeter {
            id: self.next_meter,
            kind,
            a,
            b,
            meter,
        });
        self.arm = None;
        self.choosing = None;
        self.meter_message = None;
        self.sync_meters();
        if let Some(m) = self.meters.last() {
            let through = m.meter.part.as_deref().map(|p| format!(" via {p}")).unwrap_or_default();
            let text = format!("Placed {} {} to {}{through}", m.kind.label(), m.a.label, m.b.label);
            self.log(Level::Info, Source::Meters, text, None);
        }
    }

    fn on_probe(&mut self, probe: ProbeRef, layout: &[PartPins]) {
        let Some(kind) = self.arm else {
            return;
        };
        match self.pending_a.take() {
            None => {
                self.pending_a = Some(probe);
                self.choosing = None;
                self.meter_message = None;
            }
            Some(a) => match kind {
                MeterKind::Volt => {
                    let meter = Multimeter::voltmeter(&a.node, &probe.node);
                    self.place(kind, a, probe, meter);
                }
                MeterKind::Amp => match candidates(layout, &a.node, &probe.node) {
                    Err(e) => {
                        self.log(Level::Warn, Source::Meters, e.message().to_owned(), None);
                        self.meter_message = Some(e.message().to_owned());
                    }
                    Ok(mut c) if c.len() == 1 => {
                        let meter = Multimeter::ammeter(&c.remove(0));
                        self.place(kind, a, probe, meter);
                    }
                    Ok(c) => {
                        self.choosing = Some((a, probe, c));
                        self.meter_message = None;
                    }
                },
            },
        }
    }
    /// Ideal or a real-world profile for the meters, the scope and the PSU readout.
    fn profile_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Profiles").strong());
            let b = &mut self.bench;
            profile_combo(
                ui,
                "profile_meter",
                "meters",
                &mut b.meter,
                profile::multimeters(),
                |p| &p.name,
            );
            profile_combo(ui, "profile_scope", "scope", &mut b.scope, profile::scopes(), |p| {
                &p.name
            });
            profile_combo(ui, "profile_psu", "PSU", &mut b.psu, profile::supplies(), |p| &p.name);
        });
        if let Some(note) = self.bench.scope_note(scope_scale(self.kind).2, "mA") {
            ui.weak(note);
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
                self.choosing = None;
            }
            let amp = self.arm == Some(MeterKind::Amp);
            if ui.selectable_label(amp, "Ammeter").clicked() {
                self.arm = if amp { None } else { Some(MeterKind::Amp) };
                self.pending_a = None;
                self.meter_message = None;
            }
            if self.arm.is_some() && self.choosing.is_none() {
                let step = if self.pending_a.is_none() {
                    "click point A, then point B"
                } else {
                    "click point B"
                };
                ui.weak(step);
            }
        });
        // Several parts join the two points: pick the one the ammeter goes through.
        let mut picked = None;
        let mut cancel = false;
        if let Some((a, b, cands)) = &self.choosing {
            ui.horizontal(|ui| {
                ui.weak(format!("{} to {}: through", a.label, b.label));
                for (i, c) in cands.iter().enumerate() {
                    if ui.selectable_label(false, c.refdes.as_str()).clicked() {
                        picked = Some(i);
                    }
                }
                if ui.small_button("Cancel").clicked() {
                    cancel = true;
                }
            });
        }
        if let Some(i) = picked
            && let Some((a, b, cands)) = self.choosing.take()
        {
            let meter = Multimeter::ammeter(&cands[i]);
            self.place(MeterKind::Amp, a, b, meter);
        } else if cancel {
            self.choosing = None;
            self.arm = None;
        }
        if let Some(msg) = &self.meter_message {
            ui.colored_label(LED_RED, msg);
        }
        let mut remove = None;
        for meter in &self.meters {
            ui.horizontal(|ui| {
                let reading = reading_text(&snap.meter_readings, meter.id, &self.bench);
                let through = meter
                    .meter
                    .part
                    .as_deref()
                    .map(|p| format!(" via {p}"))
                    .unwrap_or_default();
                ui.monospace(format!(
                    "{}  {} to {}{through}  {reading}",
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
            if let Some(m) = self.meters.iter().find(|m| m.id == id) {
                let text = format!("Removed {} {} to {}", m.kind.label(), m.a.label, m.b.label);
                self.log(Level::Info, Source::Meters, text, None);
            }
            self.meters.retain(|m| m.id != id);
            self.sync_meters();
        }
    }

    /// Draw the demo. The kernel runs on the worker thread.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.now = ui.input(|i| i.time);
        let mut snap = self.session.snapshot();
        // Real-world profiles: the scope's 8-bit trace and the PSU's readback.
        self.bench.scope_trace(&mut snap.led_trace, scope_scale(self.kind).2);
        let psu_band = self.bench.supply(&mut snap.supply_status);
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
        if snap.kind == self.kind {
            // Engine state changes, once each.
            if snap.booting != self.was_booting {
                self.was_booting = snap.booting;
                let text = if snap.booting {
                    "Booting the ESP32-C3 from the fetched ROM and bootloader"
                } else {
                    "Boot finished"
                };
                self.log(Level::Info, Source::Engine, text, Some(snap.time));
            }
            if snap.lagging == self.was_lagging {
                self.lag_changed_at = None;
            } else if self.now - *self.lag_changed_at.get_or_insert(self.now) >= LAG_LOG_HOLD_S {
                self.lag_changed_at = None;
                self.was_lagging = snap.lagging;
                let text = if snap.lagging {
                    format!(
                        "Running behind real time ({}); catching up",
                        trace::format_multiplier(snap.rt_factor)
                    )
                } else {
                    "Back at the requested speed".to_owned()
                };
                self.log(Level::Debug, Source::Engine, text, Some(snap.time));
            }
        }

        // One page scroll per demo, so a switch starts at the top.
        egui::ScrollArea::vertical()
            .id_salt(("demo-page", self.kind.label()))
            .show(ui, |ui| {
                ui.add_space(8.0);
                ui.horizontal_wrapped(|ui| {
                    ui.heading(RichText::new(self.kind.title()).size(30.0).strong());
                    ui.add_space(12.0);
                    let mut kind = self.kind;
                    ComboBox::from_id_salt("demo_kind")
                        .selected_text(kind.label())
                        .show_ui(ui, |ui| {
                            for k in crate::kind::PICKER {
                                ui.selectable_value(&mut kind, k, k.label());
                            }
                        });
                    if kind != self.kind {
                        self.select(kind);
                    }
                    // A new demo measures its schematic afresh (nothing cached
                    // from the one before), this frame.
                    if std::mem::take(&mut self.fits_stale) {
                        crate::schematic::reset_fits(ui.ctx());
                    }
                    ui.add_space(12.0);
                    let mut show = self.show_startup_animation;
                    if ui.checkbox(&mut show, "Show startup animation").changed() {
                        self.show_startup_animation = show;
                        let mut prefs = crate::prefs::load();
                        prefs.show_startup_animation = show;
                        if let Err(err) = crate::prefs::save(&prefs) {
                            eprintln!("Ohmforge preferences: {err}");
                            self.log(
                                Level::Error,
                                Source::Ui,
                                format!("Could not save preferences: {err}"),
                                None,
                            );
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
                        self.toggle_play(&snap);
                    }
                    if ui.add(egui::Button::new("Reset").min_size(vec2(64.0, 32.0))).clicked() {
                        self.restart();
                        self.log(Level::Info, Source::Ui, "Reset", Some(snap.time));
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
                    let resp = ui.add(slider);
                    if resp.changed() {
                        self.pot = pot;
                        self.session.send(Command::SetPot(pot));
                    }
                    if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                        let text = format!("Potentiometer set to {:.0}%", self.pot * 100.0);
                        self.log(Level::Debug, Source::Ui, text, Some(snap.time));
                    }
                }

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    let mut on = self.supply.on;
                    if ui.checkbox(&mut on, "Supply on").changed() {
                        self.supply.on = on;
                        self.session.send(Command::SetSupply(self.supply));
                        let text = if on { "Supply on" } else { "Supply off" };
                        self.log(Level::Info, Source::Psu, text, Some(snap.time));
                    }
                    let mut v = self.supply.volts;
                    let resp = ui.add(Slider::new(&mut v, 0.0..=5.0).text("Vset").suffix(" V").min_decimals(2));
                    if resp.changed() {
                        self.supply.volts = v;
                        self.session.send(Command::SetSupply(self.supply));
                    }
                    if resp.drag_stopped() || (resp.changed() && !resp.dragged()) {
                        let text = format!("PSU set to {:.2} V", self.supply.volts);
                        self.log(Level::Debug, Source::Psu, text, Some(snap.time));
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
                    let band = psu_band
                        .map(|b| format!("  (\u{b1} {:.3} V, \u{b1} {:.1} mA)", b.volts_band, b.amps_band * 1e3))
                        .unwrap_or_default();
                    ui.monospace(format!(
                        "PSU  {:.2} V  {:.2} mA  {}{band}",
                        status.volts,
                        status.display_amps() * 1e3,
                        supply::mode_label(status.mode)
                    ));
                });
                // Warnings and errors from the loaded circuit, in a strip of fixed
                // height (latched, so nothing pops in and out per frame).
                let mut current = Vec::new();
                if let Some(warn) = supply::rating_warning(&status, &self.supply, &self.rail) {
                    current.push(Message::warning(Source::Psu, warn));
                }
                // `booting` is the worker's placeholder while it builds the first sim, not an error.
                if let Some(error) = snap.error.as_ref().filter(|e| !snap.booting && e.as_str() != "booting") {
                    current.push(Message::error(
                        Source::Engine,
                        format!("Simulation paused: {error}. Change a setting or press Reset to rebuild."),
                    ));
                }
                // Only the selected demo's own snapshots speak: the first ones
                // after a switch still carry the previous demo's supply.
                if snap.kind == self.kind && !snap.booting {
                    let changes = self.status.update(self.now, &current);
                    for m in changes.appeared {
                        let level = match m.level {
                            crate::status::Level::Error => Level::Error,
                            crate::status::Level::Warning => Level::Warn,
                        };
                        self.log(level, m.source, m.text, Some(snap.time));
                    }
                    for m in changes.cleared {
                        self.log(Level::Debug, m.source, format!("Cleared: {}", m.text), Some(snap.time));
                    }
                }
                ui.add_space(4.0);
                if crate::status::strip(ui, &self.status) {
                    self.console.open();
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
                        if self.kind.has_mcu() {
                            ui.weak("seed 0 is the firmware bounce test train");
                        }
                    });
                }

                ui.add_space(8.0);
                self.profile_bar(ui);
                self.meter_bar(ui, &snap);

                ui.add_space(8.0);
                readouts(ui, &snap);

                ui.add_space(14.0);
                if snap.booting {
                    ui.label("Booting the ESP32-C3 from the fetched ROM and bootloader…");
                }
                // The panels draw whether or not the simulation runs: an error
                // pauses it and shows in the status strip; the instruments keep
                // the last good readings.
                {
                    let hit = {
                        let view = MeterView {
                            arm: self.arm,
                            pending_a: self.pending_a.as_ref().map(|p| p.id.as_str()),
                            meters: &self.meters,
                            readings: &snap.meter_readings,
                            bench: self.bench,
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
                        self.on_probe(probe, &snap.layout);
                    }
                    ui.add_space(12.0);
                    let now = SimTime::from_secs_f64(snap.time);
                    self.serial_panel.show(ui, &mut self.monitor, "UART0", now);
                    crate::digital_ui::show(ui, &snap.consoles, &snap.logic, snap.time);
                }
            });

        // The console draws over the page (nothing underneath moves).
        let sim = crate::console::SimState {
            playing: snap.playing,
            can_play: snap.error.is_none() && !snap.booting,
        };
        let shown = self.console.show(ui.ctx(), sim);
        if shown.toggle_sim && sim.can_play {
            self.toggle_play(&snap);
        }
        if shown.save_prefs {
            self.save_console_prefs();
        }

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
    let board = boards::board(kind, pos);
    match kind {
        DemoKind::Blink => blink::show_schematic(ui, &board, current, glow, snap.supply_status, meters),
        DemoKind::Dimmer | DemoKind::Fade => {
            adc_pwm::show_schematic(ui, &board, current, glow, pos, snap.supply_status, meters)
        }
        DemoKind::ButtonPoll | DemoKind::ButtonIrq => {
            button::show_schematic(ui, &board, current, glow, snap.button, snap.supply_status, meters)
        }
        // The 555 painter draws whatever the file lays out (no fixed ids).
        DemoKind::Ne555Astable | DemoKind::Ne555Monostable | DemoKind::Ne555Pwm | DemoKind::Clock555 => {
            match crate::session_sim::parsed(kind) {
                Ok(file) => {
                    let led = crate::ne555::LedState {
                        duty: snap.pwm_duty,
                        instant_ma: snap.led_instant,
                    };
                    crate::ne555::show(
                        ui,
                        file,
                        &board,
                        current,
                        glow,
                        led,
                        snap.button,
                        snap.supply_status,
                        meters,
                    )
                }
                Err(e) => {
                    ui.label(format!("{}: {e}", kind.label()));
                    SchematicHit::default()
                }
            }
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

/// The LED scope's vertical span and mA per division.
fn scope_scale(kind: DemoKind) -> (f64, f64, f64) {
    if matches!(kind, DemoKind::Blink) {
        (-4.0, 8.0, 2.0)
    } else {
        (0.0, 6.0, 1.0)
    }
}

/// A profile chooser: "Ideal" or one of `options`.
fn profile_combo<T: PartialEq + 'static>(
    ui: &mut Ui,
    salt: &str,
    label: &str,
    value: &mut Option<&'static T>,
    options: impl Iterator<Item = &'static T>,
    name: impl Fn(&'static T) -> &'static str,
) {
    ui.weak(label);
    let current = value.map_or("Ideal", &name);
    ComboBox::from_id_salt(salt).selected_text(current).show_ui(ui, |ui| {
        ui.selectable_value(value, None, "Ideal");
        for p in options {
            ui.selectable_value(value, Some(p), name(p));
        }
    });
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
    if kind.file_bench() {
        file_instruments_panel(ui, snap, time_div, session);
        return;
    }
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
        let (min_y, max_y, y_step) = scope_scale(kind);
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

/// The circuit file's own scopes and multimeters (`[[instrument]]`), for
/// the demos without an ESP32-C3: one plot per scope probe, scaled to its
/// samples, and each meter's reading.
fn file_instruments_panel(ui: &mut Ui, snap: &Snapshot, time_div: &mut f64, session: &LiveSession) {
    panel().show(ui, |ui| {
        let window = trace::visible_window(*time_div);
        let start = (snap.time - window).max(0.0);
        let end = start + window;
        for (reference, chans) in &snap.file_scopes {
            for (name, pts) in chans {
                ui.label(RichText::new(format!("{reference} \u{00b7} {name}")).strong());
                let (lo, hi) = pts
                    .iter()
                    .filter(|(t, _)| *t >= start)
                    .fold((0.0f64, 1.0f64), |(a, b), &(_, v)| (a.min(v), b.max(v)));
                let max_y = hi.ceil().max(1.0);
                let zoom = scope(
                    ui,
                    ScopePlot {
                        time: snap.time,
                        start,
                        end,
                        min_y: lo.floor().min(0.0),
                        max_y,
                        y_step: if max_y > 4.0 { 1.0 } else { 0.5 },
                        y_unit: "V",
                        x_caption: "Simulation time",
                    },
                    pts.iter().copied(),
                );
                if zoom != 0 {
                    apply_time_div(session, time_div, trace::step_time_div(*time_div, zoom));
                }
            }
        }
        for (reference, probe, text) in &snap.file_meters {
            ui.monospace(format!("{reference} \u{00b7} {probe}: {text}"));
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
mod render_tests {
    use super::*;
    use egui::Context;

    /// The console's Pause sim and the main Play/Pause share one path: it
    /// flips the worker's play state (seen in the next snapshots, which both
    /// buttons read) and logs Paused / Playing.
    #[test]
    fn toggling_play_pauses_and_resumes_the_simulation_and_logs_it() {
        let mut app = OhmforgeDemo::new();
        let wait = |app: &OhmforgeDemo, want: &dyn Fn(&Snapshot) -> bool| {
            let t0 = std::time::Instant::now();
            loop {
                let s = app.session.snapshot();
                if want(&s) {
                    return s;
                }
                assert!(
                    t0.elapsed().as_secs() < 20,
                    "timed out: kind {:?} booting {} playing {} error {:?}",
                    s.kind,
                    s.booting,
                    s.playing,
                    s.error
                );
                std::thread::sleep(Duration::from_millis(5));
            }
        };
        let s = wait(&app, &|s| s.kind == DemoKind::Blink && !s.booting && s.playing);
        app.toggle_play(&s);
        let s = wait(&app, &|s| !s.playing);
        assert_eq!(
            app.console.log.entries().last().map(|e| e.text.as_str()),
            Some("Paused")
        );
        app.toggle_play(&s);
        wait(&app, &|s| s.playing);
        assert_eq!(
            app.console.log.entries().last().map(|e| e.text.as_str()),
            Some("Playing")
        );
    }
    /// The drawing's whole painted extent, top-left corner included, lies
    /// inside its frame and inside the clip its shapes carry, and that clip
    /// starts no lower and no further right than the frame.
    fn assert_fits(what: &str, fit: &crate::schematic::FitRecord) {
        assert!(
            fit.drawn.min.x >= fit.frame.min.x - 0.5 && fit.drawn.min.y >= fit.frame.min.y - 0.5,
            "{what}: drawn {:?} starts outside its frame {:?}",
            fit.drawn,
            fit.frame
        );
        assert!(
            fit.clip.expand(0.5).contains_rect(fit.drawn),
            "{what}: drawn {:?} outside its clip {:?}",
            fit.drawn,
            fit.clip
        );
        assert!(
            fit.clip.min.y <= fit.frame.min.y + 0.5 && fit.clip.min.x <= fit.frame.min.x + 0.5,
            "{what}: clip {:?} cuts into frame {:?}",
            fit.clip,
            fit.frame
        );
    }

    /// Opening a demo after another (the app starts on blink; the clock's
    /// drawing is the widest and the PWM dimmer's reaches above its own
    /// origin) never reuses the previous drawing's fit: every demo's painted
    /// extent lies inside its frame, top included, on the full app page at a
    /// maximised window size.
    #[test]
    fn every_demo_fits_after_switching_from_another() {
        let (w, h) = (2184.0f32, 1261.0f32);
        for first in [DemoKind::Blink, DemoKind::Clock555] {
            let mut app = OhmforgeDemo::new();
            let ctx = Context::default();
            let run = |app: &mut OhmforgeDemo, n: usize| {
                let mut shapes = Vec::new();
                for _ in 0..n {
                    let raw = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(w, h))),
                        ..Default::default()
                    };
                    let mut out = ctx.run_ui(raw, |ui| {
                        egui::CentralPanel::default().show(ui, |ui| app.ui(ui));
                    });
                    out.textures_delta.clear();
                    shapes = out.shapes;
                }
                shapes
            };
            app.select(first);
            run(&mut app, 4);
            for &kind in crate::kind::PICKER.iter().filter(|k| **k != first) {
                app.select(kind);
                let shapes = run(&mut app, 4);
                let what = format!("{first:?} then {kind:?}");
                let fit = crate::schematic::last_fit(&ctx).expect("a fitted schematic");
                assert_fits(&what, &fit);
                assert!(
                    shapes.iter().any(|c| c.clip_rect == fit.clip),
                    "{what}: no shape carries the schematic's clip"
                );
                assert!(
                    fit.content.is_positive() && fit.drawn.width() > 100.0,
                    "{what}: {fit:?}"
                );
            }
        }
    }

    /// The schematic draws from any snapshot, a failed run's included (the
    /// panels never depend on the simulation succeeding), and on every demo
    /// its whole painted extent fits inside its frame, at a typical panel
    /// width, a two-column one and a small one.
    #[test]
    fn schematics_draw_unclipped_when_the_simulation_failed() {
        for &kind in crate::kind::PICKER.iter() {
            let mut snap = Snapshot::empty(kind, crate::session_sim::file_supply(kind));
            snap.error = Some("invariant: kernel: analog: time step too small (Newton did not converge)".into());
            let view = MeterView {
                arm: None,
                pending_a: None,
                meters: &[],
                readings: &[],
                bench: BenchProfiles::default(),
            };
            for width in [980.0, 470.0, 320.0] {
                let ctx = Context::default();
                let mut height = 0.0;
                // The first frame measures the drawing; the next ones fit it.
                for _ in 0..3 {
                    let raw = egui::RawInput {
                        screen_rect: Some(Rect::from_min_size(Pos2::ZERO, vec2(width, 1200.0))),
                        ..Default::default()
                    };
                    let mut output = ctx.run_ui(raw, |ui| {
                        egui::CentralPanel::default().show(ui, |ui| {
                            let top = ui.cursor().top();
                            let _ = schematic(ui, kind, 0.0, 1.0, kind.has_pot().then_some(0.5), &snap, &view);
                            height = ui.cursor().top() - top;
                        });
                    });
                    output.textures_delta.clear();
                    assert!(!output.shapes.is_empty(), "{kind:?}: nothing painted");
                }
                assert!(
                    height > 100.0,
                    "{kind:?} at {width}: the schematic took {height} points"
                );
                let fit = crate::schematic::last_fit(&ctx).expect("a fitted schematic");
                assert_fits(&format!("{kind:?} at {width}"), &fit);
                assert!(
                    fit.frame.expand(0.5).contains_rect(fit.drawn),
                    "{kind:?} at {width}: drawn {:?} outside its frame {:?}",
                    fit.drawn,
                    fit.frame
                );
                assert!(fit.drawn.right() <= width + 0.5, "{kind:?} at {width}: {:?}", fit.drawn);
            }
        }
    }
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
