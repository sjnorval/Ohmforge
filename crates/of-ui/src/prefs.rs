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
    /// When `true`, the demos are built from their circuit files
    /// (`circuits/demos`) instead of the hand-wired boards (engine E4c,
    /// preview). `--circuit-files` and `OF_CIRCUIT_FILES` turn it on for one
    /// run without changing this value.
    #[serde(default)]
    pub circuit_files: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AppPrefs {
    fn default() -> Self {
        Self {
            show_startup_animation: true,
            circuit_files: false,
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

/// True when `OF_CIRCUIT_FILES` is set to a truthy value (`1`, `true`, `yes`, or `on`).
pub fn circuit_files_by_env() -> bool {
    match std::env::var("OF_CIRCUIT_FILES") {
        Ok(value) => matches!(value.to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

/// Build the demos from their circuit files when `--circuit-files`,
/// `OF_CIRCUIT_FILES` or the saved preference asks for it.
pub fn use_circuit_files(cli_circuit_files: bool, saved: bool) -> bool {
    circuit_files_choice(cli_circuit_files, circuit_files_by_env(), saved)
}

/// Pure policy used by [`use_circuit_files`]. Any one source turns the
/// circuit files on; the hand-wired boards stay the default.
pub fn circuit_files_choice(cli: bool, env: bool, saved: bool) -> bool {
    cli || env || saved
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
            circuit_files: true,
        };
        save_to(&path, &prefs).unwrap();
        assert_eq!(load_from(&path), prefs);
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.contains("show_startup_animation"));
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
    fn circuit_files_are_off_unless_asked_for() {
        assert!(!AppPrefs::default().circuit_files);
        assert!(!circuit_files_choice(false, false, false));
        assert!(circuit_files_choice(true, false, false));
        assert!(circuit_files_choice(false, true, false));
        assert!(circuit_files_choice(false, false, true));
        // Older config files without the key keep the default.
        let path = temp_prefs_path("no-circuit-files");
        fs::write(&path, "show_startup_animation = true\n").unwrap();
        assert!(!load_from(&path).circuit_files);
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
