//! In-memory resolver (feature `engine`): files by the path written in the
//! circuit, for apps that embed their circuits and firmware (the desktop
//! demos, the web build) and for tests.

use std::borrow::Cow;
use std::collections::BTreeMap;

use crate::library::{Resolver, RomFiles};

/// Looks up a pre-booted chip: `(part, app_sha256, ready_text)`.
pub type BootedFn = dyn Fn(&str, &str, &str) -> Result<Option<of_engine::Firmware>, String> + Send + Sync;

/// Files held in memory, keyed by the path as written in the circuit file.
#[derive(Default)]
pub struct EmbeddedResolver {
    files: BTreeMap<String, Cow<'static, [u8]>>,
    rom: Option<Result<RomFiles, String>>,
    booted: Option<Box<BootedFn>>,
}

impl std::fmt::Debug for EmbeddedResolver {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("EmbeddedResolver")
            .field("files", &self.files.keys().collect::<Vec<_>>())
            .field("rom", &self.rom.as_ref().map(|r| r.is_ok()))
            .field("booted", &self.booted.is_some())
            .finish()
    }
}

impl EmbeddedResolver {
    /// No files.
    pub fn new() -> Self {
        Self::default()
    }

    /// Add a file under `path` (as written in the circuit file).
    pub fn with_file(mut self, path: &str, bytes: impl Into<Cow<'static, [u8]>>) -> Self {
        self.files.insert(path.to_owned(), bytes.into());
        self
    }

    /// Provide the mask ROM and bootloader (or the reason they are missing)
    /// for every `boot = "rom"` placement.
    pub fn with_rom(mut self, rom: Result<RomFiles, String>) -> Self {
        self.rom = Some(rom);
        self
    }

    /// Provide pre-booted chips (see [`Resolver::booted`]).
    pub fn with_booted(
        mut self,
        f: impl Fn(&str, &str, &str) -> Result<Option<of_engine::Firmware>, String> + Send + Sync + 'static,
    ) -> Self {
        self.booted = Some(Box::new(f));
        self
    }

    /// Paths of the files held, sorted.
    pub fn paths(&self) -> impl Iterator<Item = &str> {
        self.files.keys().map(String::as_str)
    }
}

impl Resolver for EmbeddedResolver {
    fn read(&self, path: &str) -> Result<Vec<u8>, String> {
        self.files
            .get(path)
            .map(|b| b.to_vec())
            .ok_or_else(|| format!("'{path}' is not embedded"))
    }

    fn rom(&self, part: &str) -> Result<RomFiles, String> {
        match &self.rom {
            Some(r) => r.clone(),
            None => Err(format!("no mask ROM is embedded for {part}")),
        }
    }

    fn booted(&self, part: &str, app_sha256: &str, ready_text: &str) -> Result<Option<of_engine::Firmware>, String> {
        match &self.booted {
            Some(f) => f(part, app_sha256, ready_text),
            None => Ok(None),
        }
    }
}
