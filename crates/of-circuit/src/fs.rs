//! Host file system (feature `fs`): reading circuit files, firmware relative
//! to the file, the parts directory and the fetched mask ROM. Not for wasm32
//! builds.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use crate::library::{PartsIndex, Resolver, RomFiles, sha256_hex};
use crate::{CircuitError, CircuitFile};

/// Suffix of circuit files found in directories.
pub const SUFFIX: &str = ".circuit.toml";

/// Resolves paths relative to a base directory (the circuit file's folder),
/// and fetched mask ROMs for the parts registered with [`FsResolver::with_rom`].
#[derive(Debug, Clone)]
pub struct FsResolver {
    base: PathBuf,
    roms: BTreeMap<String, RomSource>,
}

impl FsResolver {
    /// Resolve relative to `base`.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self {
            base: base.into(),
            roms: BTreeMap::new(),
        }
    }

    /// Look up the fetched ROM of library part `part` (for `boot = "rom"`).
    pub fn with_rom(mut self, part: &str, source: RomSource) -> Self {
        self.roms.insert(part.to_owned(), source);
        self
    }

    /// Resolve relative to the folder that holds `circuit_file`.
    pub fn for_file(circuit_file: &Path) -> Self {
        Self::new(circuit_file.parent().unwrap_or(Path::new("")))
    }

    /// The full path for `path` as written in a circuit file.
    pub fn path(&self, path: &str) -> PathBuf {
        self.base.join(path)
    }
}

impl Resolver for FsResolver {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        let full = self.path(path);
        std::fs::read(&full).map_err(|e| format!("{}: {e}", full.display()))
    }

    fn rom(&self, part: &str) -> Result<RomFiles, String> {
        let source = self
            .roms
            .get(part)
            .ok_or_else(|| format!("no mask ROM is known for {part}"))?;
        fetch_rom(source).map_err(|e| match e {
            FetchError::Missing(file) => format!(
                "{part}: {file} was not found. The mask ROM and second-stage bootloader are not bundled \
                 with Ohmforge; run scripts/fetch-esp-rom-elfs.sh (or .ps1)"
            ),
            FetchError::Invalid(msg) => msg,
        })
    }
}

/// The pinned, checksummed files of one chip's fetched ROM release
/// (`scripts/fetch-esp-rom-elfs.sh`). The chip crate owns the values.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RomSource {
    /// Release folder, for example `20260528` (`esp-rom-elfs/<release>`).
    pub release: &'static str,
    /// Mask ROM ELF file name.
    pub rom_file: &'static str,
    /// Its lower-case hex SHA-256.
    pub rom_sha256: &'static str,
    /// Second-stage bootloader file name.
    pub bootloader_file: &'static str,
    /// Its lower-case hex SHA-256.
    pub bootloader_sha256: &'static str,
}

/// Why a fetched file could not be used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FetchError {
    /// Not found in any cache folder (the file name).
    Missing(String),
    /// Found but unreadable, or not the pinned file.
    Invalid(String),
}

/// Folders searched for a fetched release, in order: `OF_ESP_ROM_DIR`,
/// `$RUNNER_TEMP/esp-rom-elfs/<release>` (CI), the repository's
/// `.cache/esp-rom-elfs/<release>`, and `.cache/esp-rom-elfs/<release>`
/// below the current directory.
pub fn rom_cache_dirs(release: &str) -> Vec<PathBuf> {
    let release = format!("esp-rom-elfs/{release}");
    let mut dirs = Vec::new();
    if let Some(dir) = std::env::var_os("OF_ESP_ROM_DIR") {
        dirs.push(PathBuf::from(dir));
    }
    if let Some(temp) = std::env::var_os("RUNNER_TEMP") {
        dirs.push(PathBuf::from(temp).join(&release));
    }
    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../../.cache")
            .join(&release),
    );
    if let Ok(cwd) = std::env::current_dir() {
        dirs.push(cwd.join(".cache").join(&release));
    }
    dirs
}

/// Read fetched `file` of `release` and check its SHA-256.
///
/// # Panics
///
/// When `OF_REQUIRE_ESP_ROM` is set and the file is missing (CI jobs that
/// fetched the ROM must not skip silently).
pub fn fetch_pinned(release: &str, file: &str, want_sha256: &str) -> Result<Vec<u8>, FetchError> {
    let Some(path) = rom_cache_dirs(release)
        .into_iter()
        .map(|d| d.join(file))
        .find(|p| p.is_file())
    else {
        if std::env::var_os("OF_REQUIRE_ESP_ROM").is_some() {
            panic!("OF_REQUIRE_ESP_ROM is set but {file} was not found; run scripts/fetch-esp-rom-elfs.sh");
        }
        return Err(FetchError::Missing(file.to_owned()));
    };
    let bytes =
        std::fs::read(&path).map_err(|e| FetchError::Invalid(format!("could not read {}: {e}", path.display())))?;
    let digest = sha256_hex(&bytes);
    if digest != want_sha256 {
        return Err(FetchError::Invalid(format!(
            "{} is not the pinned file (SHA-256 {digest}, expected {want_sha256})",
            path.display()
        )));
    }
    Ok(bytes)
}

/// The fetched mask ROM and bootloader of `source`.
pub fn fetch_rom(source: &RomSource) -> Result<RomFiles, FetchError> {
    Ok(RomFiles {
        rom: fetch_pinned(source.release, source.rom_file, source.rom_sha256)?,
        bootloader: fetch_pinned(source.release, source.bootloader_file, source.bootloader_sha256)?,
    })
}

/// Read the text of a circuit file.
pub fn read_text(path: &Path) -> Result<String, CircuitError> {
    std::fs::read_to_string(path).map_err(|e| CircuitError::general(e.to_string()).with_path(path))
}

/// Read and [`parse`](crate::parse) a circuit file; errors carry `path`.
pub fn read_circuit(path: &Path) -> Result<(String, CircuitFile), CircuitError> {
    let src = read_text(path)?;
    let file = crate::parse(&src).map_err(|e| e.with_path(path))?;
    Ok((src, file))
}

/// Expand arguments into files: a file is taken as is, a directory gives
/// every `*.circuit.toml` below it, sorted by path.
pub fn find_circuit_files(paths: &[PathBuf]) -> Result<Vec<PathBuf>, String> {
    let mut out = Vec::new();
    for p in paths {
        if p.is_file() {
            out.push(p.clone());
        } else if p.is_dir() {
            let mut found = Vec::new();
            let mut stack = vec![p.clone()];
            while let Some(d) = stack.pop() {
                let entries = std::fs::read_dir(&d).map_err(|e| format!("{}: {e}", d.display()))?;
                for e in entries {
                    let path = e.map_err(|e| format!("{}: {e}", d.display()))?.path();
                    if path.is_dir() {
                        stack.push(path);
                    } else if path.file_name().is_some_and(|n| n.to_string_lossy().ends_with(SUFFIX)) {
                        found.push(path);
                    }
                }
            }
            found.sort();
            out.extend(found);
        } else {
            return Err(format!("{}: no such file or directory", p.display()));
        }
    }
    Ok(out)
}

/// Load every `part.toml` below `dir`.
pub fn load_parts(dir: &Path) -> Result<PartsIndex, String> {
    let loaded = of_parts::load_dir(dir).map_err(|e| e.to_string())?;
    if loaded.is_empty() {
        return Err(format!("no part.toml files under {}", dir.display()));
    }
    Ok(PartsIndex::new(loaded.into_iter().map(|(_, p)| p)))
}
