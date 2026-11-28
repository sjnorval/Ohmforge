//! The slide-over panel: `~` brings it in from the right over the app (no
//! layout moves underneath), the left edge drags to resize, and the list
//! sticks to the newest entry until scrolled up.

use egui::text::{LayoutJob, TextWrapping};
use egui::{
    Align, Area, Color32, ComboBox, Context, CursorIcon, DragValue, Event, FontId, Id, Key, Layout, Order, Rect,
    RichText, ScrollArea, Sense, Stroke, TextEdit, TextFormat, UiBuilder, pos2, vec2,
};

use super::log::{Entry, EntryId, Filter, Level, Log, Settings, Source};
use crate::theme::{LED_RED, PANEL, TEAL};

/// Default and smallest panel width (points).
pub const DEFAULT_WIDTH: f32 = 560.0;
pub const MIN_WIDTH: f32 = 340.0;
const ROW_H: f32 = 18.0;
const SLIDE_S: f32 = 0.18;
const AMBER: Color32 = Color32::from_rgb(232, 168, 56);

/// The simulation's run state, for the console's own sim button. It is read
/// from the same snapshot as the main Play/Pause, so the two always agree.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct SimState {
    /// The simulation is running.
    pub playing: bool,
    /// Play/Pause is available (no error, not booting), as for the main button.
    pub can_play: bool,
}

impl SimState {
    /// The button's label and whether it is enabled.
    pub fn button(self) -> (&'static str, bool) {
        let label = if self.playing { "Pause sim" } else { "Resume sim" };
        (label, self.can_play)
    }
}

/// What a frame of the console asks the app to do.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Shown {
    /// The settings or the width changed: save the preferences.
    pub save_prefs: bool,
    /// The sim button was clicked: toggle play/pause like the main button.
    pub toggle_sim: bool,
}

/// The console: the log plus how it is viewed.
pub struct Console {
    pub log: Log,
    open: bool,
    width: f32,
    filter: Filter,
    auto_scroll: bool,
    selected: Option<EntryId>,
    show_settings: bool,
    /// Result of the last export, shown in the footer.
    note: Option<String>,
    /// This frame's sim state, and whether its button was clicked.
    sim: SimState,
    sim_clicked: bool,
    /// A Save dialog that is open, and how many entries it will write.
    saving: Option<(std::sync::mpsc::Receiver<SaveOutcome>, usize)>,
}

impl Console {
    pub fn new(settings: Settings, width: f32) -> Self {
        Self {
            log: Log::new(settings),
            open: false,
            width: width.max(MIN_WIDTH),
            filter: Filter::default(),
            auto_scroll: true,
            selected: None,
            show_settings: false,
            note: None,
            sim: SimState::default(),
            sim_clicked: false,
            saving: None,
        }
    }

    /// Record a message (see [`Log::push`]).
    pub fn push(&mut self, level: Level, source: Source, text: impl Into<String>, at: f64, sim: Option<f64>) {
        self.log.push(level, source, text, at, sim);
    }

    pub fn open(&mut self) {
        self.open = true;
    }

    pub fn width(&self) -> f32 {
        self.width
    }

    /// Handle `~` and draw the panel over everything. `sim` drives the
    /// Pause sim / Resume sim button (the log's own Pause only freezes the list).
    pub fn show(&mut self, ctx: &Context, sim: SimState) -> Shown {
        self.sim = sim;
        self.sim_clicked = false;
        self.poll_export();
        let typing = ctx.text_edit_focused();
        let (tilde, escape) = ctx.input(|i| {
            let tilde = i.events.iter().any(|e| match e {
                Event::Key {
                    key,
                    physical_key,
                    pressed: true,
                    repeat: false,
                    ..
                } => *key == Key::Backtick || *physical_key == Some(Key::Backtick),
                Event::Text(t) => t == "~" || t == "`",
                _ => false,
            });
            (tilde, i.key_pressed(Key::Escape))
        });
        if !typing && tilde {
            self.open = !self.open;
        }
        if self.open && escape && !typing {
            self.open = false;
        }
        let t = ctx.animate_bool_with_time_and_easing(
            Id::unique("of-console-slide"),
            self.open,
            SLIDE_S,
            egui::emath::easing::cubic_out,
        );
        if t <= 0.0 {
            return Shown::default();
        }
        let screen = ctx.content_rect();
        self.width = self.width.clamp(MIN_WIDTH, (screen.width() * 0.95).max(MIN_WIDTH));
        let rect = Rect::from_min_size(
            pos2(screen.right() - self.width * t, screen.top()),
            vec2(self.width, screen.height()),
        );
        let mut dirty = false;
        Area::new(Id::unique("of-console"))
            .order(Order::Foreground)
            .fixed_pos(rect.min)
            .constrain(false)
            .movable(false)
            .show(ctx, |ui| {
                // Claim the whole panel so clicks do not reach the app below.
                let bg = ui.allocate_rect(rect, Sense::click());
                let _ = bg;
                let painter = ui.painter();
                painter.rect_filled(rect, 0.0, PANEL);
                painter.vline(rect.left(), rect.y_range(), Stroke::new(1.0, TEAL.gamma_multiply(0.5)));
                // Drag the left edge to resize.
                let handle = Rect::from_min_max(rect.left_top(), pos2(rect.left() + 6.0, rect.bottom()));
                let resp = ui.interact(handle, Id::unique("of-console-resize"), Sense::drag());
                if resp.hovered() || resp.dragged() {
                    ui.ctx().set_cursor_icon(CursorIcon::ResizeHorizontal);
                    ui.painter()
                        .vline(rect.left() + 1.0, rect.y_range(), Stroke::new(2.0, TEAL));
                }
                if resp.dragged() {
                    self.width =
                        (self.width - resp.drag_delta().x).clamp(MIN_WIDTH, (screen.width() * 0.95).max(MIN_WIDTH));
                }
                if resp.drag_stopped() {
                    dirty = true;
                }
                let inner = rect.shrink2(vec2(14.0, 12.0));
                ui.scope_builder(UiBuilder::new().max_rect(inner), |ui| {
                    dirty |= self.contents(ui);
                });
            });
        Shown {
            save_prefs: dirty,
            toggle_sim: std::mem::take(&mut self.sim_clicked),
        }
    }

    fn contents(&mut self, ui: &mut egui::Ui) -> bool {
        let mut dirty = false;
        ui.horizontal(|ui| {
            ui.label(RichText::new("Log").strong().size(18.0));
            ui.weak(format!("{} entries", self.log.len()));
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if ui.button("Close").on_hover_text("Close (~ or Esc)").clicked() {
                    self.open = false;
                }
                ui.toggle_value(&mut self.show_settings, "Settings");
            });
        });
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            if self.log.is_paused() {
                let n = self.log.new_since_pause();
                let label = RichText::new(format!("Resume  +{n} new")).color(Color32::from_rgb(12, 35, 31));
                if ui.add(egui::Button::new(label).fill(TEAL)).clicked() {
                    self.log.resume();
                }
            } else if ui
                .button("Pause")
                .on_hover_text("Freeze the list; new entries keep counting")
                .clicked()
            {
                self.log.pause();
            }
            let (label, enabled) = self.sim.button();
            if ui
                .add_enabled(enabled, egui::Button::new(label))
                .on_hover_text("Pause or resume the simulation (same as the main Play/Pause)")
                .clicked()
            {
                self.sim_clicked = true;
            }
            if ui.button("Clear").clicked() {
                self.log.clear();
                self.selected = None;
            }
            ui.checkbox(&mut self.auto_scroll, "Auto-scroll")
                .on_hover_text("Follow the newest entry; scrolling up stops it until you are back at the bottom");
            ui.separator();
            let idle = self.saving.is_none();
            if ui
                .add_enabled(idle, egui::Button::new("Export .txt"))
                .on_hover_text("Save the listed entries as text")
                .clicked()
            {
                self.export(ui.ctx(), false);
            }
            if ui
                .add_enabled(idle, egui::Button::new("Export .json"))
                .on_hover_text("Save the listed entries as JSON")
                .clicked()
            {
                self.export(ui.ctx(), true);
            }
        });
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ComboBox::from_id_salt("of-console-level")
                .selected_text(format!("{} and up", self.filter.min_level.name()))
                .width(110.0)
                .show_ui(ui, |ui| {
                    for l in Level::ALL {
                        ui.selectable_value(&mut self.filter.min_level, l, format!("{} and up", l.name()));
                    }
                });
            for s in Source::ALL {
                let mut on = self.filter.shows_source(s);
                if ui.toggle_value(&mut on, s.label()).changed() {
                    self.filter.set_source(s, on);
                }
            }
            ui.add(
                TextEdit::singleline(&mut self.filter.search)
                    .hint_text("Search")
                    .desired_width(140.0),
            );
        });
        if self.show_settings {
            ui.add_space(4.0);
            dirty |= self.settings_ui(ui);
        }
        ui.add_space(6.0);

        let footer_h = 20.0;
        let rows: Vec<&Entry> = self.log.visible(&self.filter).collect();
        let list_h = (ui.available_height() - footer_h - 6.0).max(ROW_H * 3.0);
        let mut clicked = None;
        ScrollArea::vertical()
            .id_salt("of-console-list")
            .auto_shrink([false, false])
            .max_height(list_h)
            .min_scrolled_height(list_h)
            .stick_to_bottom(self.auto_scroll)
            .show_rows(ui, ROW_H, rows.len(), |ui, range| {
                for e in &rows[range] {
                    if row(ui, e, self.selected == Some(e.id)) {
                        clicked = Some(e.id);
                    }
                }
            });
        if let Some(id) = clicked {
            self.selected = if self.selected == Some(id) { None } else { Some(id) };
        }
        ui.add_space(6.0);
        let footer = match (&self.note, self.selected.and_then(|id| self.log.get(id))) {
            (Some(n), _) => n.clone(),
            (None, Some(e)) => format!(
                "Selected #{}  {}{}",
                e.id.0,
                e.sim.map(|t| format!("sim {:.6} s  ", t)).unwrap_or_default(),
                if e.count > 1 {
                    format!("\u{d7}{} until {:.3} s", e.count, e.last_at)
                } else {
                    String::new()
                }
            ),
            (None, None) if self.log.is_empty() => "Nothing logged yet \u{b7} ~ to close".to_owned(),
            (None, None) => format!(
                "{} shown \u{b7} ~ to close \u{b7} click an entry to select it",
                rows.len()
            ),
        };
        ui.add_sized(
            vec2(ui.available_width(), footer_h),
            egui::Label::new(RichText::new(footer).weak()).truncate(),
        );
        dirty
    }

    fn settings_ui(&mut self, ui: &mut egui::Ui) -> bool {
        let mut s = self.log.settings();
        crate::theme::panel()
            .fill(Color32::from_rgb(30, 35, 43))
            .show(ui, |ui| {
                egui::Grid::new("of-console-settings")
                    .num_columns(2)
                    .spacing([12.0, 6.0])
                    .show(ui, |ui| {
                        ui.label("Max entries");
                        ui.add(
                            DragValue::new(&mut s.max_entries)
                                .range(Settings::MIN_ENTRIES..=Settings::MAX_ENTRIES)
                                .speed(100.0),
                        );
                        ui.end_row();
                        ui.label("Record levels");
                        ComboBox::from_id_salt("of-console-capture")
                            .selected_text(format!("{} and up", s.capture_level.name()))
                            .show_ui(ui, |ui| {
                                for l in Level::ALL {
                                    ui.selectable_value(&mut s.capture_level, l, format!("{} and up", l.name()));
                                }
                            });
                        ui.end_row();
                        ui.label("Snapshot memory");
                        ui.add(DragValue::new(&mut s.snapshot_budget_mb).range(8..=4096).suffix(" MB"));
                        ui.end_row();
                        ui.label("Snapshot every");
                        ui.add(
                            DragValue::new(&mut s.snapshot_interval_ms)
                                .range(10..=10_000)
                                .suffix(" ms"),
                        );
                        ui.end_row();
                    });
                ui.weak("Snapshots are for stepping back to an entry's state (coming next).");
            });
        if s != self.log.settings() {
            self.log.set_settings(s);
            return true;
        }
        false
    }

    /// Ask where to save the listed entries. The dialog runs on its own
    /// thread, so the app keeps drawing; [`Console::poll_export`] picks up
    /// the result. Cancelling does nothing.
    fn export(&mut self, ctx: &Context, json: bool) {
        if self.saving.is_some() {
            return;
        }
        let rows: Vec<&Entry> = self.log.visible(&self.filter).collect();
        let count = rows.len();
        let (body, ext) = if json {
            (Log::to_json(rows.iter().copied()), "json")
        } else {
            (Log::to_text(rows.iter().copied()), "txt")
        };
        match save_with_dialog(ctx, body, ext) {
            Ok(rx) => self.saving = Some((rx, count)),
            Err(e) => self.note = Some(format!("Export failed: {e}")),
        }
    }

    /// Take the Save dialog's result once it is in.
    fn poll_export(&mut self) {
        let Some((rx, count)) = &self.saving else {
            return;
        };
        let outcome = match rx.try_recv() {
            Ok(outcome) => outcome,
            Err(std::sync::mpsc::TryRecvError::Empty) => return,
            Err(std::sync::mpsc::TryRecvError::Disconnected) => Some(Err("the Save dialog closed unexpectedly".into())),
        };
        match outcome {
            None => {}
            Some(Ok(path)) => self.note = Some(format!("Saved {count} entries to {}", path.display())),
            Some(Err(e)) => self.note = Some(format!("Export failed: {e}")),
        }
        self.saving = None;
    }
}

/// What the Save dialog did: `None` when cancelled, else the written file or the error.
type SaveOutcome = Option<Result<std::path::PathBuf, String>>;

/// Default file name for an export made at `unix` seconds.
fn export_file_name(unix: u64, ext: &str) -> String {
    format!("ohmforge-log-{unix}.{ext}")
}

/// The Save dialog's starting folder: `logs` in the Ohmforge config folder.
#[cfg(not(target_arch = "wasm32"))]
fn export_dir() -> std::path::PathBuf {
    crate::prefs::config_dir()
        .map(|d| d.join("logs"))
        .unwrap_or_else(|| std::path::PathBuf::from("."))
}

#[cfg(not(target_arch = "wasm32"))]
fn save_with_dialog(
    ctx: &Context,
    body: String,
    ext: &'static str,
) -> std::io::Result<std::sync::mpsc::Receiver<SaveOutcome>> {
    let (tx, rx) = std::sync::mpsc::channel();
    let dir = export_dir();
    let unix = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let name = export_file_name(unix, ext);
    let ctx = ctx.clone();
    let run = move || {
        // The default folder may not exist yet; the dialog still opens if this fails.
        let _ = std::fs::create_dir_all(&dir);
        let kind = if ext == "json" { "JSON" } else { "Text" };
        let picked = rfd::FileDialog::new()
            .set_title("Export log")
            .set_directory(&dir)
            .set_file_name(name)
            .add_filter(kind, &[ext])
            .save_file();
        let outcome = picked.map(|path| std::fs::write(&path, body).map(|()| path).map_err(|e| e.to_string()));
        let _ = tx.send(outcome);
        ctx.request_repaint();
    };
    // macOS shows dialogs from the main thread only (this blocks until it closes).
    #[cfg(target_os = "macos")]
    run();
    #[cfg(not(target_os = "macos"))]
    std::thread::Builder::new()
        .name("of-console-export".into())
        .spawn(run)?;
    Ok(rx)
}

#[cfg(target_arch = "wasm32")]
fn save_with_dialog(
    _ctx: &Context,
    _body: String,
    _ext: &'static str,
) -> std::io::Result<std::sync::mpsc::Receiver<SaveOutcome>> {
    Err(std::io::Error::other("export is desktop-only for now"))
}

fn level_colour(ui: &egui::Ui, l: Level) -> Color32 {
    match l {
        Level::Error => LED_RED,
        Level::Warn => AMBER,
        Level::Info => ui.visuals().text_color(),
        Level::Debug | Level::Trace => ui.visuals().weak_text_color(),
    }
}

/// One entry on one line; returns `true` when clicked.
fn row(ui: &mut egui::Ui, e: &Entry, selected: bool) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(ui.available_width(), ROW_H), Sense::click());
    let painter = ui.painter_at(rect);
    if selected {
        painter.rect_filled(rect, 3.0, TEAL.gamma_multiply(0.22));
    } else if resp.hovered() {
        painter.rect_filled(rect, 3.0, ui.visuals().faint_bg_color);
    }
    let weak = ui.visuals().weak_text_color();
    let mono = FontId::monospace(12.0);
    let y = rect.center().y;
    let mut x = rect.left() + 4.0;
    let mut cell = |text: String, colour: Color32, w: f32| {
        let g = painter.layout_no_wrap(text, mono.clone(), colour);
        painter.galley(pos2(x, y - g.size().y * 0.5), g, colour);
        x += w;
    };
    cell(format!("{:>9.3}", e.at), weak, 76.0);
    cell(e.level.name().to_uppercase(), level_colour(ui, e.level), 48.0);
    cell(e.source.name().to_owned(), weak, 56.0);
    let count_w = if e.count > 1 {
        let g = painter.layout_no_wrap(format!("\u{d7}{}", e.count), mono.clone(), TEAL);
        let w = g.size().x;
        painter.galley(pos2(rect.right() - 4.0 - w, y - g.size().y * 0.5), g, TEAL);
        w + 10.0
    } else {
        0.0
    };
    let colour = level_colour(ui, e.level);
    let mut job = LayoutJob::single_section(
        e.text.clone(),
        TextFormat {
            font_id: FontId::proportional(13.0),
            color: colour,
            ..Default::default()
        },
    );
    job.wrap = TextWrapping {
        max_width: (rect.right() - 4.0 - count_w - x).max(0.0),
        max_rows: 1,
        break_anywhere: true,
        overflow_character: Some('\u{2026}'),
    };
    let g = painter.layout_job(job);
    painter.galley(pos2(x, y - g.size().y * 0.5), g, colour);
    resp.on_hover_text(&e.text).clicked()
}

#[cfg(test)]
mod tests {
    use super::{SimState, export_file_name};

    #[test]
    fn exports_default_to_a_timestamped_name() {
        assert_eq!(export_file_name(1_764_000_000, "txt"), "ohmforge-log-1764000000.txt");
        assert_eq!(export_file_name(0, "json"), "ohmforge-log-0.json");
    }

    #[test]
    fn the_sim_button_follows_the_simulation_state() {
        let running = SimState {
            playing: true,
            can_play: true,
        };
        let paused = SimState {
            playing: false,
            can_play: true,
        };
        let failed = SimState {
            playing: false,
            can_play: false,
        };
        assert_eq!(running.button(), ("Pause sim", true));
        assert_eq!(paused.button(), ("Resume sim", true));
        // Disabled exactly when the main Play/Pause is (error or booting).
        assert_eq!(failed.button(), ("Resume sim", false));
    }
}
