//! Host file system (feature `fs`): reading circuit files, firmware relative
//! to the file, and the parts directory. Not for wasm32 builds.

use std::path::{Path, PathBuf};

use crate::library::{PartsIndex, Resolver};
use crate::{CircuitError, CircuitFile};

/// Suffix of circuit files found in directories.
pub const SUFFIX: &str = ".circuit.toml";

/// Resolves paths relative to a base directory (the circuit file's folder).
#[derive(Debug, Clone)]
pub struct FsResolver {
    base: PathBuf,
}

impl FsResolver {
    /// Resolve relative to `base`.
    pub fn new(base: impl Into<PathBuf>) -> Self {
        Self { base: base.into() }
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
