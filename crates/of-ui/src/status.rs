//! The desktop's status strip: the current warnings and errors (supply
//! ratings from the parts on the PSU's + rail, simulation errors), each
//! latched for [`LATCH_S`] after its condition clears so that nothing pops
//! in and out per frame. The strip itself always takes the same height.

use crate::console::log::Source;

/// How long a message stays listed after its condition last held (s).
pub const LATCH_S: f64 = 1.5;

/// How serious a message is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Level {
    Warning,
    Error,
}

/// One warning or error, worded from the loaded circuit.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Message {
    pub level: Level,
    /// Where it comes from, for the log console.
    pub source: Source,
    pub text: String,
}

/// What [`StatusLog::update`] changed: messages that appeared, and ones
/// that dropped off after their latch time.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Changes {
    pub appeared: Vec<Message>,
    pub cleared: Vec<Message>,
}

impl Message {
    pub fn warning(source: Source, text: impl Into<String>) -> Self {
        Self {
            level: Level::Warning,
            source,
            text: text.into(),
        }
    }

    pub fn error(source: Source, text: impl Into<String>) -> Self {
        Self {
            level: Level::Error,
            source,
            text: text.into(),
        }
    }
}

#[derive(Clone, Debug)]
struct Entry {
    msg: Message,
    first: f64,
    seen: f64,
}

/// The latched messages.
#[derive(Clone, Debug, Default)]
pub struct StatusLog {
    entries: Vec<Entry>,
}

impl StatusLog {
    /// `current` holds at time `now` (s); a message not in it stays until
    /// [`LATCH_S`] after it was last seen.
    /// The changes go to the log console, once each (not per frame).
    pub fn update(&mut self, now: f64, current: &[Message]) -> Changes {
        let mut changes = Changes::default();
        for m in current {
            match self.entries.iter_mut().find(|e| e.msg == *m) {
                Some(e) => e.seen = now,
                None => {
                    changes.appeared.push(m.clone());
                    self.entries.push(Entry {
                        msg: m.clone(),
                        first: now,
                        seen: now,
                    });
                }
            }
        }
        self.entries.retain(|e| {
            let keep = now - e.seen <= LATCH_S;
            if !keep {
                changes.cleared.push(e.msg.clone());
            }
            keep
        });
        changes
    }

    /// Forget everything (another circuit was loaded).
    pub fn clear(&mut self) {
        self.entries.clear();
    }

    /// Errors first, then warnings, oldest first within each.
    pub fn messages(&self) -> Vec<&Message> {
        let mut v: Vec<&Entry> = self.entries.iter().collect();
        v.sort_by(|a, b| b.msg.level.cmp(&a.msg.level).then(a.first.total_cmp(&b.first)));
        v.into_iter().map(|e| &e.msg).collect()
    }

    /// (warnings, errors).
    pub fn counts(&self) -> (usize, usize) {
        let e = self.entries.iter().filter(|e| e.msg.level == Level::Error).count();
        (self.entries.len() - e, e)
    }

    /// The message the strip shows: the newest of the most serious level.
    pub fn headline(&self) -> Option<&Message> {
        self.entries
            .iter()
            .max_by(|a, b| a.msg.level.cmp(&b.msg.level).then(a.first.total_cmp(&b.first)))
            .map(|e| &e.msg)
    }
}

/// Height of the strip (points); reserved even when it is empty.
#[cfg(feature = "gui")]
pub const STRIP_HEIGHT: f32 = 24.0;

/// Draw the strip: a count badge and the headline on one truncated line,
/// every message in the tooltip. Returns `true` when clicked (the caller
/// opens the log console, so nothing below the strip moves).
#[cfg(feature = "gui")]
pub fn strip(ui: &mut egui::Ui, log: &StatusLog) -> bool {
    use egui::text::{LayoutJob, TextWrapping};
    use egui::{Color32, FontId, Sense, TextFormat, vec2};

    use crate::theme::LED_RED;

    const AMBER: Color32 = Color32::from_rgb(232, 168, 56);
    let colour = |l: Level| if l == Level::Error { LED_RED } else { AMBER };
    let (warnings, errors) = log.counts();
    let (rect, response) = ui.allocate_exact_size(vec2(ui.available_width(), STRIP_HEIGHT), Sense::click());
    let painter = ui.painter_at(rect);
    let muted = ui.visuals().weak_text_color();
    painter.rect_filled(rect, 4.0, ui.visuals().faint_bg_color);
    let badge = match (errors, warnings) {
        (0, 0) => None,
        (0, w) => Some((format!("\u{26a0} {w} warning{}", if w == 1 { "" } else { "s" }), AMBER)),
        (e, 0) => Some((format!("\u{26a0} {e} error{}", if e == 1 { "" } else { "s" }), LED_RED)),
        (e, w) => Some((
            format!(
                "\u{26a0} {e} error{} \u{00b7} {w} warning{}",
                if e == 1 { "" } else { "s" },
                if w == 1 { "" } else { "s" }
            ),
            LED_RED,
        )),
    };
    let mut x = rect.left() + 8.0;
    let y = rect.center().y;
    match &badge {
        Some((text, c)) => {
            let g = painter.layout_no_wrap(text.clone(), FontId::proportional(13.0), *c);
            let w = g.size().x;
            painter.galley(egui::pos2(x, y - g.size().y * 0.5), g, *c);
            x += w + 12.0;
        }
        None => {
            let g = painter.layout_no_wrap("No warnings".into(), FontId::proportional(13.0), muted);
            painter.galley(egui::pos2(x, y - g.size().y * 0.5), g, muted);
        }
    }
    if let Some(m) = log.headline() {
        let mut job = LayoutJob::single_section(
            m.text.clone(),
            TextFormat {
                font_id: FontId::proportional(13.0),
                color: colour(m.level),
                ..Default::default()
            },
        );
        job.wrap = TextWrapping {
            max_width: (rect.right() - 8.0 - x).max(0.0),
            max_rows: 1,
            break_anywhere: true,
            overflow_character: Some('\u{2026}'),
        };
        let g = painter.layout_job(job);
        painter.galley(egui::pos2(x, y - g.size().y * 0.5), g, colour(m.level));
    }
    let all = log.messages();
    response
        .on_hover_ui(|ui| {
            for m in &all {
                ui.colored_label(colour(m.level), &m.text);
            }
            ui.weak("Click to open the log (~)");
        })
        .on_hover_cursor(egui::CursorIcon::PointingHand)
        .clicked()
}

#[cfg(test)]
mod tests {
    use super::{LATCH_S, Level, Message, StatusLog};
    use crate::console::log::Source;

    /// A message stays for the latch time after its condition clears, a
    /// repeat does not add a second entry, and errors lead.
    #[test]
    fn messages_latch_and_do_not_toggle_per_frame() {
        let mut log = StatusLog::default();
        let w = Message::warning(Source::Psu, "PSU low");
        let e = Message::error(Source::Engine, "Simulation paused");
        log.update(0.0, std::slice::from_ref(&w));
        // The condition flickers off and on every frame: still one entry.
        for k in 1..60 {
            let now = f64::from(k) / 60.0;
            let cur = if k % 2 == 0 { vec![w.clone()] } else { vec![] };
            log.update(now, &cur);
            assert_eq!(log.counts(), (1, 0), "frame {k}");
        }
        log.update(1.0, &[w.clone(), e.clone()]);
        assert_eq!(log.counts(), (1, 1));
        assert_eq!(log.headline().map(|m| m.level), Some(Level::Error));
        assert_eq!(log.messages()[0], &e);
        // Cleared at 1.0 s: listed until LATCH_S later, then gone.
        log.update(1.0 + LATCH_S - 0.01, &[]);
        assert_eq!(log.counts(), (1, 1));
        log.update(1.0 + LATCH_S + 0.01, &[]);
        assert_eq!(log.counts(), (0, 0));
        assert!(log.headline().is_none());
    }

    /// Each message reaches the log once when it appears and once when it
    /// drops off, however often its condition flickers in between.
    #[test]
    fn changes_report_each_message_once() {
        let mut log = StatusLog::default();
        let w = Message::warning(Source::Psu, "PSU set to 3.30 V is outside the rated supply of U2");
        let c = log.update(0.0, std::slice::from_ref(&w));
        assert_eq!(c.appeared, vec![w.clone()]);
        for k in 1..30 {
            let cur = if k % 2 == 0 { vec![w.clone()] } else { vec![] };
            let c = log.update(f64::from(k) / 60.0, &cur);
            assert!(c.appeared.is_empty() && c.cleared.is_empty(), "frame {k}");
        }
        let c = log.update(10.0, &[]);
        assert_eq!(c.cleared, vec![w]);
    }
}
