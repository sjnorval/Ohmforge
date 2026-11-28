//! Persistent desktop preferences (config file in the platform config directory).
//!
//! eframe storage is not enabled in this build, so preferences are stored as a
//! small TOML file using crates already in the workspace (`serde`, `toml`).

use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

/// On-disk desktop preferences.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AppPrefs {
    /// When `true`, the desktop app plays the EILIi load screen on startup.
    ///
    /// `--no-splash` and `OF_NO_SPLASH` still skip the splash for that run
    /// without changing this value.
    #[serde(default = "default_true")]
    pub show_startup_animation: bool,
    /// The `~` log console.
    #[serde(default)]
    pub console: ConsolePrefs,
}

/// Log console settings (see [`crate::console::log::Settings`]).
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct ConsolePrefs {
    /// Ring buffer size (entries).
    pub max_entries: usize,
    /// Memory for state snapshots (MB), for time travel.
    pub snapshot_budget_mb: u32,
    /// Interval between state snapshots (ms).
    pub snapshot_interval_ms: u32,
    /// The least serious level recorded: trace, debug, info, warn or error.
    pub capture_level: String,
    /// Panel width (points).
    pub width: u32,
}

impl Default for ConsolePrefs {
    fn default() -> Self {
        Self::from_settings(crate::console::log::Settings::default(), crate::console::DEFAULT_WIDTH)
    }
}

impl ConsolePrefs {
    /// Settings and panel width as stored.
    pub fn from_settings(s: crate::console::log::Settings, width: f32) -> Self {
        Self {
            max_entries: s.max_entries,
            snapshot_budget_mb: s.snapshot_budget_mb,
            snapshot_interval_ms: s.snapshot_interval_ms,
            capture_level: s.capture_level.name().to_owned(),
            width: width.round().max(0.0) as u32,
        }
    }

    /// The settings; an unknown level falls back to the default.
    pub fn settings(&self) -> crate::console::log::Settings {
        let d = crate::console::log::Settings::default();
        crate::console::log::Settings {
            max_entries: self.max_entries,
            snapshot_budget_mb: self.snapshot_budget_mb,
            snapshot_interval_ms: self.snapshot_interval_ms,
            capture_level: crate::console::log::Level::from_name(&self.capture_level).unwrap_or(d.capture_level),
        }
    }

    /// Panel width (points).
    pub fn width(&self) -> f32 {
        self.width as f32
    }
}

fn default_true() -> bool {
    true
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            show_startup_animation: true,
            console: ConsolePrefs::default(),
        }
    }
}

/// Directory that holds [`config_path`], if the platform gives us one.
pub fn config_dir() -> Option<PathBuf> {
    #[cfg(target_arch = "wasm32")]
    {
        None
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        native_config_dir()
    }
}

/// `config.toml` inside [`config_dir`].
pub fn config_path() -> Option<PathBuf> {
    config_dir().map(|dir| dir.join("config.toml"))
}

#[cfg(not(target_arch = "wasm32"))]
fn native_config_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    {
        let appdata = std::env::var_os("APPDATA")?;
        Some(PathBuf::from(appdata).join("Ohmforge"))
    }
    #[cfg(target_os = "macos")]
    {
        let home = std::env::var_os("HOME")?;
        Some(PathBuf::from(home).join("Library/Application Support/Ohmforge"))
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    {
        if let Some(xdg) = std::env::var_os("XDG_CONFIG_HOME") {
            Some(PathBuf::from(xdg).join("ohmforge"))
        } else {
            let home = std::env::var_os("HOME")?;
            Some(PathBuf::from(home).join(".config/ohmforge"))
        }
    }
}

/// Load preferences from `path`, or the defaults when the file is missing or invalid.
pub fn load_from(path: &Path) -> AppPrefs {
    let Ok(text) = fs::read_to_string(path) else {
        return AppPrefs::default();
    };
    match toml::from_str::<AppPrefs>(&text) {
        Ok(prefs) => prefs,
        Err(err) => {
            eprintln!("Ohmforge preferences: {err} ({})", path.display());
            AppPrefs::default()
        }
    }
}

/// Load from the platform config path, or the defaults.
pub fn load() -> AppPrefs {
    match config_path() {
        Some(path) => load_from(&path),
        None => AppPrefs::default(),
    }
}

/// Write `prefs` to `path`, creating the parent directory when needed.
pub fn save_to(path: &Path, prefs: &AppPrefs) -> std::io::Result<()> {
    if let Some(parent) = path.parent()
        && !parent.as_os_str().is_empty()
    {
        fs::create_dir_all(parent)?;
    }
    let text = toml::to_string_pretty(prefs).map_err(std::io::Error::other)?;
    fs::write(path, text)
}

/// Save to the platform config path.
pub fn save(prefs: &AppPrefs) -> std::io::Result<()> {
    let Some(path) = config_path() else {
        return Err(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "no platform config directory",
        ));
    };
    save_to(&path, prefs)
}

/// Play the splash unless a command-line or environment override blocks it, or
/// the user has turned the persistent setting off.
pub fn should_play_splash(cli_no_splash: bool, show_startup_animation: bool) -> bool {
    splash_choice(cli_no_splash, crate::splash::blocked_by_env(), show_startup_animation)
}

/// Pure policy used by [`should_play_splash`]. `env_blocked` is [`crate::splash::blocked_by_env`].
pub fn splash_choice(cli_no_splash: bool, env_blocked: bool, show_startup_animation: bool) -> bool {
    !cli_no_splash && !env_blocked && show_startup_animation
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_prefs_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let dir = std::env::temp_dir().join(format!("ohmforge-prefs-test-{name}-{nanos}"));
        fs::create_dir_all(&dir).unwrap();
        dir.join("config.toml")
    }

    #[test]
    fn missing_file_uses_defaults() {
        let path = temp_prefs_path("missing");
        assert!(!path.exists());
        assert_eq!(load_from(&path), AppPrefs::default());
        assert!(AppPrefs::default().show_startup_animation);
    }

    #[test]
    fn round_trip_false() {
        let path = temp_prefs_path("roundtrip");
        let prefs = AppPrefs {
            show_startup_animation: false,
            ..AppPrefs::default()
        };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path), prefs);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("show_startup_animation"));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn console_settings_round_trip_and_old_files_get_defaults() {
        use crate::console::log::{Level, Settings};
        let path = temp_prefs_path("console");
        let s = Settings {
            max_entries: 2_500,
            snapshot_budget_mb: 128,
            snapshot_interval_ms: 250,
            capture_level: Level::Warn,
        };
        let prefs = AppPrefs {
            console: ConsolePrefs::from_settings(s, 640.0),
            ..AppPrefs::default()
        };
        save_to(&path, &prefs).unwrap();
        let back = load_from(&path);
        assert_eq!(back.console.settings(), s);
        assert_eq!(back.console.width(), 640.0);
        // A file from before the console has none of its keys.
        fs::write(&path, "show_startup_animation = false\n").unwrap();
        assert_eq!(load_from(&path).console.settings(), Settings::default());
        // An unknown level keeps the rest.
        fs::write(&path, "[console]\nmax_entries = 500\ncapture_level = \"loud\"\n").unwrap();
        let c = load_from(&path).console.settings();
        assert_eq!(
            (c.max_entries, c.capture_level),
            (500, Settings::default().capture_level)
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn invalid_file_uses_defaults() {
        let path = temp_prefs_path("invalid");
        fs::write(&path, "this is not toml {").unwrap();
        assert_eq!(load_from(&path), AppPrefs::default());
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn unknown_keys_are_ignored() {
        let path = temp_prefs_path("extra");
        fs::write(&path, "show_startup_animation = false\nfuture_flag = 1\n").unwrap();
        assert!(!load_from(&path).show_startup_animation);
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn the_old_circuit_files_key_is_ignored() {
        // E4c's preview setting; the circuit files are the only path since E4d.
        let path = temp_prefs_path("old-circuit-files");
        fs::write(&path, "show_startup_animation = false\ncircuit_files = true\n").unwrap();
        assert_eq!(
            load_from(&path),
            AppPrefs {
                show_startup_animation: false,
                ..AppPrefs::default()
            }
        );
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(path.parent().unwrap());
    }

    #[test]
    fn flags_override_the_setting_without_needing_it_off() {
        assert!(splash_choice(false, false, true));
        assert!(!splash_choice(true, false, true));
        assert!(!splash_choice(false, true, true));
        assert!(!splash_choice(false, false, false));
        assert!(!splash_choice(true, true, true));
        assert!(!splash_choice(true, false, false));
    }
}
