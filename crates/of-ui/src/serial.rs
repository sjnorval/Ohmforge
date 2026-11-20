//! Serial monitor console panel for egui.

use egui::{Color32, Frame, Key, Margin, RichText, ScrollArea, TextEdit, Ui};

use of_core::SimTime;
use of_instruments::{Delivery, Dir, SerialMonitor};

const PANEL: Color32 = Color32::from_rgb(22, 26, 32);

fn panel() -> Frame {
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

fn copy_dump(monitor: &SerialMonitor) -> String {
    monitor
        .display_lines()
        .iter()
        .map(|(at, dir, text)| format_line(*at, *dir, text))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Lean serial console: log view, host send, clear and copy.
pub struct SerialMonitorPanel {
    draft: String,
    /// Stick to bottom unless the user scrolled up.
    follow: bool,
}

impl Default for SerialMonitorPanel {
    fn default() -> Self {
        Self {
            draft: String::new(),
            follow: true,
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

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("Copy").clicked() {
                        ui.ctx().copy_text(copy_dump(monitor));
                    }
                    if ui.button("Clear").clicked() {
                        monitor.clear();
                        self.draft.clear();
                        self.follow = true;
                    }
                });
            });

            ui.add_space(6.0);

            let stick = self.follow;
            let output = ScrollArea::vertical()
                .id_salt("serial_monitor_log")
                .min_scrolled_height(140.0)
                .animated(false)
                .stick_to_bottom(stick)
                .show(ui, |ui| {
                    for (at, dir, text) in monitor.display_lines() {
                        ui.monospace(format_line(at, dir, &text));
                    }
                });

            if stick {
                let max_scroll = (output.content_size.y - output.inner_rect.height()).max(0.0);
                let at_bottom = output.state.clamped_offset().y >= max_scroll - 1.0;
                if !at_bottom {
                    self.follow = false;
                }
            }

            if monitor.delivery() == Delivery::QueuedOnly {
                ui.weak("Send queues bytes — UART RX not modelled yet");
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
