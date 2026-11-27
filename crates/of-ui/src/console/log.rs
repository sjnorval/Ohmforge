//! The log behind the `~` console: a ring buffer of entries, each with a
//! stable [`EntryId`] (ids keep counting after old entries are dropped, so
//! a later state snapshot can be keyed by the entry it belongs to), plus
//! collapsing of repeats, filters, pause and export. No egui here, so the
//! behaviour is testable without the gui feature.

use std::collections::VecDeque;
use std::fmt::Write as _;

/// How serious an entry is (ordered: `Trace` < ... < `Error`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    Trace,
    Debug,
    Info,
    Warn,
    Error,
}

impl Level {
    pub const ALL: [Level; 5] = [Level::Trace, Level::Debug, Level::Info, Level::Warn, Level::Error];

    /// Lower-case name, as used in exports and the settings file.
    pub fn name(self) -> &'static str {
        match self {
            Level::Trace => "trace",
            Level::Debug => "debug",
            Level::Info => "info",
            Level::Warn => "warn",
            Level::Error => "error",
        }
    }

    pub fn from_name(s: &str) -> Option<Level> {
        Level::ALL.into_iter().find(|l| l.name().eq_ignore_ascii_case(s.trim()))
    }
}

/// Where an entry came from.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Source {
    /// The simulation engine (solver errors, boot, real-time lag).
    Engine,
    /// The bench supply and the ratings of the parts on its rail.
    Psu,
    /// Parts and their circuit-file data.
    Parts,
    /// The meters and scope.
    Meters,
    /// Firmware output on the UART.
    Firmware,
    /// User actions and the app itself.
    Ui,
}

impl Source {
    pub const ALL: [Source; 6] = [
        Source::Engine,
        Source::Psu,
        Source::Parts,
        Source::Meters,
        Source::Firmware,
        Source::Ui,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Source::Engine => "engine",
            Source::Psu => "psu",
            Source::Parts => "parts",
            Source::Meters => "meters",
            Source::Firmware => "uart",
            Source::Ui => "ui",
        }
    }

    /// Label for the filter buttons.
    pub fn label(self) -> &'static str {
        match self {
            Source::Engine => "Engine",
            Source::Psu => "PSU",
            Source::Parts => "Parts",
            Source::Meters => "Meters",
            Source::Firmware => "UART",
            Source::Ui => "UI",
        }
    }

    fn bit(self) -> u8 {
        1 << (self as u8)
    }
}

/// Stable id of an entry: never reused, even after the ring buffer drops it.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct EntryId(pub u64);

/// One logged message; a repeat of the newest entry bumps its `count`.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub id: EntryId,
    pub level: Level,
    pub source: Source,
    pub text: String,
    /// App time of the first occurrence (s since start).
    pub at: f64,
    /// Simulated time of the first occurrence (s), when a simulation ran.
    pub sim: Option<f64>,
    /// How many times it was logged in a row.
    pub count: u32,
    /// App time of the latest repeat.
    pub last_at: f64,
}

/// Console settings (persisted with the desktop preferences).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Settings {
    /// Ring buffer size; the oldest entries go first.
    pub max_entries: usize,
    /// Memory for state snapshots (MB); used by time travel (next release).
    pub snapshot_budget_mb: u32,
    /// Interval between state snapshots (ms); used by time travel.
    pub snapshot_interval_ms: u32,
    /// Entries below this level are not recorded at all.
    pub capture_level: Level,
}

impl Settings {
    pub const MIN_ENTRIES: usize = 100;
    pub const MAX_ENTRIES: usize = 1_000_000;
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            max_entries: 10_000,
            snapshot_budget_mb: 64,
            snapshot_interval_ms: 100,
            capture_level: Level::Debug,
        }
    }
}

/// What the list shows.
#[derive(Clone, Debug, PartialEq)]
pub struct Filter {
    /// The least serious level shown.
    pub min_level: Level,
    sources: u8,
    /// Case-insensitive text search (empty: everything).
    pub search: String,
}

impl Default for Filter {
    fn default() -> Self {
        Self {
            min_level: Level::Trace,
            sources: u8::MAX,
            search: String::new(),
        }
    }
}

impl Filter {
    pub fn shows_source(&self, s: Source) -> bool {
        self.sources & s.bit() != 0
    }

    pub fn set_source(&mut self, s: Source, on: bool) {
        if on {
            self.sources |= s.bit();
        } else {
            self.sources &= !s.bit();
        }
    }

    pub fn matches(&self, e: &Entry) -> bool {
        if e.level < self.min_level || !self.shows_source(e.source) {
            return false;
        }
        let needle = self.search.trim();
        needle.is_empty() || e.text.to_lowercase().contains(&needle.to_lowercase())
    }
}

/// The log.
#[derive(Clone, Debug, Default)]
pub struct Log {
    entries: VecDeque<Entry>,
    next_id: u64,
    settings: Settings,
    /// Newest id visible while paused, and how many entries arrived since.
    paused: Option<(Option<EntryId>, usize)>,
}

impl Log {
    pub fn new(settings: Settings) -> Self {
        let mut log = Self::default();
        log.set_settings(settings);
        log
    }

    pub fn settings(&self) -> Settings {
        self.settings
    }

    /// Apply new settings; a smaller buffer drops the oldest entries now.
    pub fn set_settings(&mut self, mut s: Settings) {
        s.max_entries = s.max_entries.clamp(Settings::MIN_ENTRIES, Settings::MAX_ENTRIES);
        self.settings = s;
        self.trim();
    }

    fn trim(&mut self) {
        while self.entries.len() > self.settings.max_entries {
            self.entries.pop_front();
        }
    }

    /// Record a message at app time `at` (s). A repeat of the newest entry
    /// (same level, source and text) bumps its count instead. Returns the
    /// entry's id, or `None` when its level is below the capture level.
    pub fn push(
        &mut self,
        level: Level,
        source: Source,
        text: impl Into<String>,
        at: f64,
        sim: Option<f64>,
    ) -> Option<EntryId> {
        if level < self.settings.capture_level {
            return None;
        }
        let text = text.into();
        if let Some((_, new)) = &mut self.paused {
            *new += 1;
        }
        if let Some(last) = self.entries.back_mut()
            && last.level == level
            && last.source == source
            && last.text == text
        {
            last.count = last.count.saturating_add(1);
            last.last_at = at;
            return Some(last.id);
        }
        let id = EntryId(self.next_id);
        self.next_id += 1;
        self.entries.push_back(Entry {
            id,
            level,
            source,
            text,
            at,
            sim,
            count: 1,
            last_at: at,
        });
        self.trim();
        Some(id)
    }

    /// Drop every entry (ids keep counting).
    pub fn clear(&mut self) {
        self.entries.clear();
        if let Some(p) = &mut self.paused {
            *p = (None, 0);
        }
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// Every entry, oldest first (filters and pause ignored).
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn entries(&self) -> impl DoubleEndedIterator<Item = &Entry> + ExactSizeIterator {
        self.entries.iter()
    }

    pub fn get(&self, id: EntryId) -> Option<&Entry> {
        let first = self.entries.front()?.id.0;
        let i = usize::try_from(id.0.checked_sub(first)?).ok()?;
        self.entries.get(i).filter(|e| e.id == id)
    }

    /// Freeze the view; entries still arrive and are counted.
    pub fn pause(&mut self) {
        if self.paused.is_none() {
            self.paused = Some((self.entries.back().map(|e| e.id), 0));
        }
    }

    pub fn resume(&mut self) {
        self.paused = None;
    }

    pub fn is_paused(&self) -> bool {
        self.paused.is_some()
    }

    /// Messages logged since the pause (repeats included).
    pub fn new_since_pause(&self) -> usize {
        self.paused.map_or(0, |(_, n)| n)
    }

    /// The entries the list shows: those matching `filter` and, while
    /// paused, no newer than the pause.
    pub fn visible<'a>(&'a self, filter: &'a Filter) -> impl Iterator<Item = &'a Entry> + 'a {
        // While paused: up to the newest entry at the pause (none if cleared since).
        let frozen = self.paused.map(|(id, _)| id);
        self.entries.iter().filter(move |e| {
            let in_view = match frozen {
                None => true,
                Some(None) => false,
                Some(Some(upto)) => e.id <= upto,
            };
            in_view && filter.matches(e)
        })
    }

    /// Plain text, one line per entry.
    pub fn to_text<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> String {
        let mut out = String::new();
        for e in entries {
            let _ = write!(out, "[{:>10.3} s]", e.at);
            match e.sim {
                Some(t) => {
                    let _ = write!(out, " [sim {:.6} s]", t);
                }
                None => out.push_str(" [sim        -  ]"),
            }
            let _ = write!(
                out,
                " {:<5} {:<6} {}",
                e.level.name().to_uppercase(),
                e.source.name(),
                e.text
            );
            if e.count > 1 {
                let _ = write!(out, " (\u{d7}{})", e.count);
            }
            out.push('\n');
        }
        out
    }

    /// A JSON array of entry objects.
    pub fn to_json<'a>(entries: impl IntoIterator<Item = &'a Entry>) -> String {
        let mut out = String::from("[");
        for (i, e) in entries.into_iter().enumerate() {
            if i > 0 {
                out.push(',');
            }
            out.push_str("\n  {");
            let _ = write!(
                out,
                "\"id\": {}, \"level\": \"{}\", \"source\": \"{}\", \"at_s\": {}, \"sim_s\": {}, \"count\": {}, \"text\": ",
                e.id.0,
                e.level.name(),
                e.source.name(),
                json_num(e.at),
                e.sim.map_or_else(|| "null".to_owned(), json_num),
                e.count
            );
            json_str(&mut out, &e.text);
            out.push('}');
        }
        out.push_str("\n]\n");
        out
    }
}

fn json_num(v: f64) -> String {
    if v.is_finite() {
        format!("{v}")
    } else {
        "null".to_owned()
    }
}

fn json_str(out: &mut String, s: &str) {
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                let _ = write!(out, "\\u{:04x}", c as u32);
            }
            c => out.push(c),
        }
    }
    out.push('"');
}

#[cfg(test)]
mod tests {
    use super::{EntryId, Filter, Level, Log, Settings, Source};

    fn log_with(max: usize) -> Log {
        Log::new(Settings {
            max_entries: max,
            capture_level: Level::Trace,
            ..Settings::default()
        })
    }

    #[test]
    fn the_buffer_keeps_the_newest_entries_and_ids_keep_counting() {
        let mut log = log_with(100);
        for k in 0..250 {
            log.push(Level::Info, Source::Ui, format!("m{k}"), f64::from(k), None);
        }
        assert_eq!(log.len(), 100);
        let ids: Vec<u64> = log.entries().map(|e| e.id.0).collect();
        assert_eq!(ids.first(), Some(&150));
        assert_eq!(ids.last(), Some(&249));
        assert_eq!(log.get(EntryId(200)).map(|e| e.text.as_str()), Some("m200"));
        assert!(log.get(EntryId(10)).is_none());
        // Shrinking the buffer drops the oldest at once; clearing keeps ids unique.
        log.set_settings(Settings {
            max_entries: 100,
            ..log.settings()
        });
        log.clear();
        assert!(log.is_empty());
        assert_eq!(
            log.push(Level::Info, Source::Ui, "after", 0.0, None),
            Some(EntryId(250))
        );
    }

    #[test]
    fn a_tiny_buffer_size_is_raised_to_the_minimum() {
        let log = log_with(3);
        assert_eq!(log.settings().max_entries, Settings::MIN_ENTRIES);
    }

    #[test]
    fn repeats_in_a_row_collapse_with_a_count() {
        let mut log = log_with(100);
        let a = log.push(Level::Warn, Source::Psu, "PSU low for U1", 1.0, Some(0.1));
        let b = log.push(Level::Warn, Source::Psu, "PSU low for U1", 2.0, Some(0.2));
        log.push(Level::Warn, Source::Psu, "PSU low for U1", 3.0, None);
        assert_eq!(a, b);
        assert_eq!(log.len(), 1);
        let e = log.entries().next().unwrap();
        assert_eq!((e.count, e.at, e.last_at, e.sim), (3, 1.0, 3.0, Some(0.1)));
        // Another level, source or text starts a new entry; so does a repeat after it.
        log.push(Level::Error, Source::Psu, "PSU low for U1", 4.0, None);
        log.push(Level::Warn, Source::Engine, "PSU low for U1", 5.0, None);
        log.push(Level::Warn, Source::Psu, "PSU low for U1", 6.0, None);
        assert_eq!(log.len(), 4);
    }

    #[test]
    fn entries_below_the_capture_level_are_not_recorded() {
        let mut log = Log::new(Settings {
            capture_level: Level::Info,
            ..Settings::default()
        });
        assert_eq!(log.push(Level::Debug, Source::Engine, "x", 0.0, None), None);
        assert!(log.push(Level::Info, Source::Engine, "y", 0.0, None).is_some());
        assert_eq!(log.len(), 1);
    }

    #[test]
    fn filters_by_level_source_and_text() {
        let mut log = log_with(100);
        log.push(Level::Debug, Source::Engine, "step rescued", 0.0, None);
        log.push(
            Level::Warn,
            Source::Psu,
            "PSU 3.30 V below U2 ti.ne555 4.5-16 V",
            0.0,
            None,
        );
        log.push(Level::Error, Source::Engine, "Simulation paused: singular", 0.0, None);
        log.push(Level::Info, Source::Firmware, "boot ok", 0.0, None);
        let shown = |f: &Filter| log.visible(f).map(|e| e.text.clone()).collect::<Vec<_>>();
        let mut f = Filter::default();
        assert_eq!(shown(&f).len(), 4);
        f.min_level = Level::Warn;
        assert_eq!(shown(&f).len(), 2);
        f.set_source(Source::Psu, false);
        assert_eq!(shown(&f), vec!["Simulation paused: singular"]);
        f = Filter::default();
        f.search = "NE555".into();
        assert_eq!(shown(&f).len(), 1);
        f.search = "  ".into();
        assert_eq!(shown(&f).len(), 4);
    }

    #[test]
    fn pause_freezes_the_view_and_counts_what_arrives() {
        let mut log = log_with(100);
        log.push(Level::Info, Source::Ui, "a", 0.0, None);
        log.pause();
        log.push(Level::Info, Source::Ui, "b", 1.0, None);
        log.push(Level::Info, Source::Ui, "b", 1.5, None);
        log.push(Level::Info, Source::Ui, "c", 2.0, None);
        let f = Filter::default();
        assert_eq!(log.new_since_pause(), 3);
        assert_eq!(log.visible(&f).map(|e| e.text.as_str()).collect::<Vec<_>>(), vec!["a"]);
        log.resume();
        assert_eq!(log.new_since_pause(), 0);
        assert_eq!(log.visible(&f).count(), 3);
        // Clearing while paused shows nothing until resumed.
        log.pause();
        log.clear();
        log.push(Level::Info, Source::Ui, "d", 3.0, None);
        assert_eq!(log.visible(&f).count(), 0);
        assert_eq!(log.new_since_pause(), 1);
    }

    #[test]
    fn exports_text_and_json() {
        let mut log = log_with(100);
        log.push(Level::Warn, Source::Psu, "R1 \"hot\"\n", 1.5, Some(0.25));
        log.push(Level::Warn, Source::Psu, "R1 \"hot\"\n", 1.6, Some(0.26));
        log.push(Level::Info, Source::Ui, "Loaded", 2.0, None);
        let text = Log::to_text(log.entries());
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 3, "{text}");
        assert!(lines[0].starts_with("[     1.500 s] [sim 0.250000 s] WARN  psu    R1 \"hot\""));
        assert!(lines[1].ends_with("(\u{d7}2)"));
        assert!(lines[2].contains("INFO  ui     Loaded"));
        let json = Log::to_json(log.entries());
        assert!(json.starts_with("[\n  {\"id\": 0, \"level\": \"warn\", \"source\": \"psu\""));
        assert!(json.contains("\"at_s\": 1.5, \"sim_s\": 0.25, \"count\": 2, \"text\": \"R1 \\\"hot\\\"\\n\"}"));
        assert!(json.contains("\"sim_s\": null"));
        assert!(json.trim_end().ends_with(']'));
        assert_eq!(Log::to_json([]), "[\n]\n");
    }

    #[test]
    fn level_names_round_trip() {
        for l in Level::ALL {
            assert_eq!(Level::from_name(l.name()), Some(l));
        }
        assert_eq!(Level::from_name(" WARN "), Some(Level::Warn));
        assert_eq!(Level::from_name("loud"), None);
    }
}
