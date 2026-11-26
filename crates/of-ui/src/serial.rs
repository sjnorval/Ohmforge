//! Serial monitor console panel for egui.

use egui::{Color32, Frame, Key, Margin, RichText, ScrollArea, TextEdit, Ui};

use of_core::SimTime;
use of_instruments::{Delivery, Dir, SerialMonitor};

const PANEL: Color32 = Color32::from_rgb(22, 26, 32);
const HIT: Color32 = Color32::from_rgb(24, 160, 150);
const HEX_WIDTH: usize = 16;

pub(crate) fn panel() -> Frame {
    Frame::new().fill(PANEL).corner_radius(8).inner_margin(Margin::same(14))
}

fn arrow(dir: Dir) -> &'static str {
    match dir {
        Dir::DeviceToHost => "←",
        Dir::HostToDevice => "→",
    }
}

fn format_line(at: SimTime, dir: Dir, text: &str) -> String {
    let time = format!("[{:.3} ms]", at.as_secs_f64() * 1e3);
    format!("{time} {} {text}", arrow(dir))
}

fn copy_dump(monitor: &SerialMonitor, hex: bool) -> String {
    if hex {
        monitor
            .hex_lines(HEX_WIDTH)
            .into_iter()
            .map(|(at, dir, off, bytes)| {
                format!(
                    "[{:.3} ms] {} {}",
                    at.as_secs_f64() * 1e3,
                    arrow(dir),
                    SerialMonitor::format_hex_line(off, &bytes, HEX_WIDTH)
                )
            })
            .collect::<Vec<_>>()
            .join("\n")
    } else {
        monitor
            .display_lines()
            .iter()
            .map(|(at, dir, text)| format_line(*at, *dir, text))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// Lean serial console: log view, host send, clear, copy, hex and find.
pub struct SerialMonitorPanel {
    draft: String,
    /// Stick to bottom unless the user scrolled up.
    follow: bool,
    /// Show a hex dump instead of the text lines.
    hex: bool,
    find: String,
    find_idx: usize,
}

impl Default for SerialMonitorPanel {
    fn default() -> Self {
        Self {
            draft: String::new(),
            follow: true,
            hex: false,
            find: String::new(),
            find_idx: 0,
        }
    }
}

impl SerialMonitorPanel {
    /// Draws the serial monitor inside the current UI region.
    pub fn show(&mut self, ui: &mut Ui, monitor: &mut SerialMonitor, title: &str, sim_now: SimTime) {
        panel().show(ui, |ui| {
            ui.set_min_width(ui.available_width());

            ui.horizontal(|ui| {
                ui.label(RichText::new(title).strong());
                ui.label("·");
                let baud = monitor
                    .baud()
                    .map(|b| format!("{}", b.round() as u64))
                    .unwrap_or_else(|| "—".to_owned());
                ui.label(baud);

                ui.checkbox(&mut self.hex, "Hex");

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(copy_dump(monitor, self.hex));
                    }
                    if ui.button("Clear").clicked() {
                        monitor.clear();
                        self.draft.clear();
                        self.find.clear();
                        self.find_idx = 0;
                        self.follow = true;
                    }
                });
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label("Find");
                let prev = self.find.clone();
                let resp = ui.add(
                    TextEdit::singleline(&mut self.find)
                        .desired_width(180.0)
                        .hint_text("text or hex…"),
                );
                if self.find != prev {
                    self.find_idx = 0;
                }
                let hits = monitor.find(&self.find);
                if !hits.is_empty() {
                    self.find_idx %= hits.len();
                    ui.label(format!("{} / {}", self.find_idx + 1, hits.len()));
                    let next = ui.button("Next").clicked()
                        || (resp.has_focus() && ui.input(|i| i.key_pressed(Key::Enter) || i.key_pressed(Key::F3)));
                    let prev_hit = ui.button("Prev").clicked()
                        || (resp.has_focus() && ui.input(|i| i.modifiers.shift && i.key_pressed(Key::F3)));
                    if next {
                        self.find_idx = (self.find_idx + 1) % hits.len();
                    }
                    if prev_hit {
                        self.find_idx = (self.find_idx + hits.len() - 1) % hits.len();
                    }
                } else if !self.find.trim().is_empty() {
                    ui.weak("no match");
                }
            });

            ui.add_space(6.0);

            let hits = monitor.find(&self.find);
            let current = hits.get(self.find_idx).copied();

            let stick = self.follow;
            let output = ScrollArea::vertical()
                .id_salt("serial_monitor_log")
                .min_scrolled_height(140.0)
                .animated(false)
                .stick_to_bottom(stick)
                .show(ui, |ui| {
                    if self.hex {
                        for (at, dir, off, bytes) in monitor.hex_lines(HEX_WIDTH) {
                            let text = format_line(at, dir, &SerialMonitor::format_hex_line(off, &bytes, HEX_WIDTH));
                            let hit = current.is_some_and(|h| h.at == at && h.dir == dir);
                            ui.monospace(if hit {
                                RichText::new(text).color(HIT)
                            } else {
                                RichText::new(text)
                            });
                        }
                    } else {
                        for (at, dir, text) in monitor.display_lines() {
                            let line = format_line(at, dir, &text);
                            let hit = current.is_some_and(|h| h.at == at && h.dir == dir);
                            ui.monospace(if hit {
                                RichText::new(line).color(HIT)
                            } else {
                                RichText::new(line)
                            });
                        }
                    }
                });

            if stick {
                let max_scroll = (output.content_size.y - output.inner_rect.height()).max(0.0);
                let at_bottom = output.state.clamped_offset().y >= max_scroll - 1.0;
                if !at_bottom {
                    self.follow = false;
                }
            }

            match monitor.delivery() {
                Delivery::QueuedOnly => {
                    ui.weak("Send queues bytes — UART RX not modelled yet");
                }
                Delivery::Attached => {
                    ui.weak("Send delivers to UART0 RX at the configured baud");
                }
            }

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                let response = ui.add(
                    TextEdit::singleline(&mut self.draft)
                        .desired_width(ui.available_width() - 64.0)
                        .hint_text("Type to send…"),
                );
                let send_clicked = ui.button("Send").clicked();
                let enter =
                    ui.input(|input| input.key_pressed(Key::Enter)) && (response.has_focus() || response.lost_focus());

                if (send_clicked || enter) && !self.draft.trim().is_empty() {
                    let mut payload = self.draft.clone();
                    if !payload.ends_with('\n') {
                        payload.push('\n');
                    }
                    let _ = monitor.try_send(sim_now, payload.as_bytes());
                    self.draft.clear();
                    self.follow = true;
                }
            });
        });
    }
}
