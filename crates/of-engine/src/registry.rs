//! Model registry: `model` strings from `part.toml` to component constructors
//! (E4b, engine plan Task 9).

use std::any::Any;
use std::collections::BTreeMap;
use std::fmt;

use of_component_api::{Component, ElabError, ParamValue};

/// Firmware resolved for an MCU placement. `of-engine` stays chip-agnostic:
/// the chip crate interprets the bytes (or downcasts [`Firmware::Booted`]).
pub enum Firmware {
    /// An ELF loaded straight into RAM / flash mapping.
    Elf(Vec<u8>),
    /// A flash app booted through the chip's mask ROM and a second-stage
    /// bootloader (never bundled: the caller fetches them).
    Rom {
        /// Application image (`.bin`) written at the app partition.
        app: Vec<u8>,
        /// Partition table.
        partitions: Vec<u8>,
        /// Mask ROM ELF.
        rom: Vec<u8>,
        /// Second-stage bootloader ELF.
        bootloader: Vec<u8>,
        /// UART text that marks the end of boot (for example the app banner).
        ready_text: Option<String>,
    },
    /// A chip that is already booted (for example from a desktop cache). The
    /// chip crate downcasts it to its SoC type.
    Booted(Box<dyn Any + Send>),
}

impl fmt::Debug for Firmware {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Elf(b) => write!(f, "Elf({} bytes)", b.len()),
            Self::Rom { app, ready_text, .. } => {
                write!(f, "Rom {{ app: {} bytes, ready_text: {ready_text:?} }}", app.len())
            }
            Self::Booted(_) => f.write_str("Booted(..)"),
        }
    }
}

/// What a constructor gets for one placement.
pub struct ComponentSpec<'a> {
    /// Reference designator, for example `U1`.
    pub refdes: &'a str,
    /// Library part id, for example `espressif.esp32-c3`.
    pub part_id: &'a str,
    /// Typed parameters from the circuit file.
    pub params: &'a BTreeMap<String, ParamValue>,
    /// Resolved firmware (MCUs only).
    pub firmware: Option<Firmware>,
    /// Electrical pins this placement connects, in file order.
    pub pins: &'a [String],
    /// Per-component seed.
    pub seed: u64,
}

impl ComponentSpec<'_> {
    /// Numeric parameters only (numbers and flags), lower-cased keys.
    pub fn numeric_params(&self) -> BTreeMap<String, f64> {
        self.params
            .iter()
            .filter_map(|(k, v)| v.as_f64().map(|x| (k.to_ascii_lowercase(), x)))
            .collect()
    }

    /// One parameter, matched without regard to case.
    pub fn param(&self, name: &str) -> Option<&ParamValue> {
        self.params.get(name).or_else(|| {
            self.params
                .iter()
                .find(|(k, _)| k.eq_ignore_ascii_case(name))
                .map(|(_, v)| v)
        })
    }
}

/// Constructor for one model.
pub type Ctor = Box<dyn Fn(ComponentSpec<'_>) -> Result<Box<dyn Component>, ElabError> + Send + Sync>;

/// Model string (`rust:led`) to constructor.
#[derive(Default)]
pub struct Registry {
    ctors: BTreeMap<String, Ctor>,
}

impl fmt::Debug for Registry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Registry")
            .field("models", &self.models().collect::<Vec<_>>())
            .finish()
    }
}

impl Registry {
    /// An empty registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Register `model`. Registering the same model twice is a bug and panics.
    pub fn register(&mut self, model: &str, ctor: Ctor) {
        assert!(
            self.ctors.insert(model.to_owned(), ctor).is_none(),
            "model '{model}' registered twice"
        );
    }

    /// True if `model` has a constructor.
    pub fn contains(&self, model: &str) -> bool {
        self.ctors.contains_key(model)
    }

    /// Registered model strings, sorted.
    pub fn models(&self) -> impl Iterator<Item = &str> {
        self.ctors.keys().map(String::as_str)
    }

    /// Build one component.
    pub fn build(&self, model: &str, spec: ComponentSpec<'_>) -> Result<Box<dyn Component>, ElabError> {
        let ctor = self.ctors.get(model).ok_or_else(|| {
            ElabError::msg(format!(
                "{}: no model '{model}' is registered (part {})",
                spec.refdes, spec.part_id
            ))
        })?;
        ctor(spec)
    }
}
