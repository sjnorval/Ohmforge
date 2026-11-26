//! Serde types for `*.circuit.toml` (RFC 0003, format = 1).
//!
//! Values that an error can point at are [`Spanned`], so validation reports a
//! line and column. `Spanned` compares by value only; use
//! [`same_content`](crate::same_content) to compare whole files.

use std::collections::BTreeMap;

use serde::Deserialize;
use toml::Spanned;

/// Newest format this crate reads and writes.
pub const FORMAT_VERSION: u32 = 1;

/// A whole circuit file.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircuitFile {
    /// Schema version; must be `1..=FORMAT_VERSION`.
    pub format: Spanned<u32>,
    /// Name, ground net, description.
    pub circuit: CircuitMeta,
    /// Solver and kernel settings.
    #[serde(default)]
    pub sim: SimSettings,
    /// Placed parts, in file order.
    #[serde(default, rename = "component")]
    pub components: Vec<ComponentEntry>,
    /// Schematic-only symbols (ground, later power flags).
    #[serde(default, rename = "symbol")]
    pub symbols: Vec<SymbolEntry>,
    /// Nets in file order (router order input).
    #[serde(default, rename = "net")]
    pub nets: Vec<NetEntry>,
    /// Schematic-wide data (keepouts).
    #[serde(default)]
    pub schematic: SchematicSettings,
    /// Optional measurement points (consumed by E5).
    #[serde(default, rename = "probe")]
    pub probes: Vec<ProbeEntry>,
    /// Optional instruments on the probes (RFC 0004, E5e; additive, `format` stays 1).
    #[serde(default, rename = "instrument")]
    pub instruments: Vec<InstrumentEntry>,
}

impl CircuitFile {
    /// The `[[probe]]` with this name (exact match).
    pub fn probe(&self, name: &str) -> Option<&ProbeEntry> {
        self.probes.iter().find(|p| p.name() == name)
    }
}

/// `[circuit]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CircuitMeta {
    /// Display name.
    pub name: String,
    /// The one reference net (default `gnd`).
    #[serde(default = "default_ground")]
    pub ground: Spanned<String>,
    /// Free text.
    #[serde(default)]
    pub description: Option<String>,
}

/// Name of the ground net when `[circuit] ground` is left out.
pub const DEFAULT_GROUND: &str = "gnd";

fn default_ground() -> Spanned<String> {
    Spanned::new(0..0, DEFAULT_GROUND.into())
}

/// Process corner for part parameters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum SimCorner {
    /// Typical values.
    #[default]
    Typ,
    /// Minimum values.
    Min,
    /// Maximum values.
    Max,
}

impl SimCorner {
    /// The file spelling (`typ`, `min`, `max`).
    pub fn as_str(self) -> &'static str {
        match self {
            SimCorner::Typ => "typ",
            SimCorner::Min => "min",
            SimCorner::Max => "max",
        }
    }
}

/// `[sim]`; the defaults reproduce today's blink demo.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields, default)]
pub struct SimSettings {
    /// Kernel quantum (µs).
    pub quantum_us: u64,
    /// Analog max step (s).
    pub max_step: f64,
    /// Parameter corner.
    pub corner: SimCorner,
    /// Ambient temperature (°C); `None` keeps the solver default.
    pub temperature_c: Option<f64>,
    /// Base seed; per-component seeds default to this.
    pub seed: u64,
    /// Start from initial conditions instead of an operating point: every
    /// net at 0 V unless listed in `ic`, the supplies applied at t = 0 (a
    /// cold power-up; needed for circuits with no DC operating point, such
    /// as a 555 astable). Optional; `false` by default.
    pub uic: bool,
    /// Net voltages at t = 0 when `uic` is set (net name to volts).
    pub ic: BTreeMap<String, f64>,
}

impl Default for SimSettings {
    fn default() -> Self {
        Self {
            quantum_us: 50,
            max_step: 5e-6,
            corner: SimCorner::Typ,
            temperature_c: None,
            seed: 0,
            uic: false,
            ic: BTreeMap::new(),
        }
    }
}

/// A typed parameter value.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(untagged)]
pub enum ParamValue {
    /// Number (TOML integers are read as `f64`).
    Number(f64),
    /// Boolean.
    Bool(bool),
    /// Text, for example a mode name.
    Text(String),
    /// List of names, for example `inputs = ["GPIO9"]`.
    List(Vec<String>),
}

impl ParamValue {
    /// The value as a number, as the engine's `ElabCtx::param` sees it
    /// (`true` is 1, `false` is 0); `None` for text and lists.
    pub fn as_number(&self) -> Option<f64> {
        match self {
            ParamValue::Number(v) => Some(*v),
            ParamValue::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
            ParamValue::Text(_) | ParamValue::List(_) => None,
        }
    }
}

/// One `[[component]]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentEntry {
    /// Reference designator.
    #[serde(rename = "ref")]
    pub reference: Spanned<String>,
    /// Library part id.
    pub part: Spanned<String>,
    /// Schematic origin.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    /// Parameters (sorted, deterministic).
    #[serde(default)]
    pub params: BTreeMap<String, ParamValue>,
    /// Per-component seed (defaults to `[sim] seed`).
    #[serde(default)]
    pub seed: Option<u64>,
    /// Firmware reference (MCUs only).
    #[serde(default)]
    pub firmware: Option<FirmwareRef>,
    /// Instance geometry (until E6 symbols).
    #[serde(default)]
    pub schematic: Option<ComponentSchematic>,
}

impl ComponentEntry {
    /// The ref as a plain str.
    pub fn reference(&self) -> &str {
        self.reference.get_ref()
    }

    /// The part id as a plain str.
    pub fn part_id(&self) -> &str {
        self.part.get_ref()
    }
}

/// Firmware by path and hash, never bytes.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FirmwareRef {
    /// App image (ELF for `direct`, flash `.bin` for `rom`), relative to the circuit file, `/` separated.
    pub app: Spanned<String>,
    /// Lower-case hex SHA-256 of `app`.
    pub app_sha256: Spanned<String>,
    /// Boot path.
    pub boot: BootKind,
    /// Partition table (`rom` boot).
    #[serde(default)]
    pub partitions: Option<Spanned<String>>,
    /// SHA-256 of `partitions`.
    #[serde(default)]
    pub partitions_sha256: Option<Spanned<String>>,
    /// Boot is complete once UART0 has printed this text (also the pre-boot cache key).
    #[serde(default)]
    pub ready_text: Option<String>,
}

/// How the chip starts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BootKind {
    /// Direct ELF load.
    Direct,
    /// Real mask ROM + second-stage bootloader (fetched, never committed).
    Rom,
}

impl BootKind {
    /// The file spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            BootKind::Direct => "direct",
            BootKind::Rom => "rom",
        }
    }
}

/// Net kind; drives router order and stroke.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NetKind {
    /// Signal net.
    #[default]
    Signal,
    /// Supply rail.
    Power,
    /// The ground net.
    Ground,
}

impl NetKind {
    /// The file spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            NetKind::Signal => "signal",
            NetKind::Power => "power",
            NetKind::Ground => "ground",
        }
    }
}

/// One `[[net]]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetEntry {
    /// Net id (solver node name).
    pub name: Spanned<String>,
    /// Display label (defaults to the name).
    #[serde(default)]
    pub label: Option<String>,
    /// Kind.
    #[serde(default)]
    pub kind: NetKind,
    /// `REF.PIN` terminals in router order.
    pub pins: Vec<Spanned<String>>,
    /// Editor waypoints (empty until wire-routing PR 6).
    #[serde(default)]
    pub waypoints: Vec<[f32; 2]>,
}

impl NetEntry {
    /// The net name as a plain str.
    pub fn name(&self) -> &str {
        self.name.get_ref()
    }

    /// Terminals as str.
    pub fn pins(&self) -> impl Iterator<Item = &str> {
        self.pins.iter().map(|p| p.get_ref().as_str())
    }
}

/// Pin departure direction.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
pub enum Dir {
    /// North.
    N,
    /// East.
    E,
    /// South.
    S,
    /// West.
    W,
}

impl Dir {
    /// The file spelling.
    pub fn as_str(self) -> &'static str {
        match self {
            Dir::N => "N",
            Dir::E => "E",
            Dir::S => "S",
            Dir::W => "W",
        }
    }
}

/// A pin that slides with a parameter (pot wiper): its position is
/// `at + (to - at) * param`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Slide {
    /// Position at param = 1, relative to the component `at`.
    pub to: [f32; 2],
    /// Component parameter in 0..=1 (0.5 when the component does not set it).
    pub param: Spanned<String>,
}

/// One schematic pin.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchematicPin {
    /// Electrical pin name.
    pub pin: Spanned<String>,
    /// Schematic pin id (defaults to `pin`).
    #[serde(default)]
    pub id: Option<String>,
    /// Display name (defaults to `pin`).
    #[serde(default)]
    pub name: Option<String>,
    /// Position relative to `at` (at param = 0 for sliding pins).
    pub at: [f32; 2],
    /// Stub direction.
    pub dir: Dir,
    /// Stub only (`Pin { routed: false }`); connects by name.
    #[serde(default)]
    pub stub: bool,
    /// Sliding pin.
    #[serde(default)]
    pub slide: Option<Slide>,
}

/// Instance geometry.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ComponentSchematic {
    /// Schematic id (defaults to the lower-case ref).
    #[serde(default)]
    pub id: Option<String>,
    /// Printed reference (defaults to the ref).
    #[serde(default)]
    pub label: Option<String>,
    /// Routing body `[x0, y0, x1, y1]` relative to `at`.
    pub body: [f32; 4],
    /// Pins.
    pub pins: Vec<SchematicPin>,
}

/// Symbol kinds format 1 knows.
pub const SYMBOL_KINDS: &[&str] = &["ground"];

/// One `[[symbol]]`: schematic only, no electrical model.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SymbolEntry {
    /// Ref (shares the component namespace).
    #[serde(rename = "ref")]
    pub reference: Spanned<String>,
    /// One of [`SYMBOL_KINDS`].
    pub kind: Spanned<String>,
    /// Schematic id.
    #[serde(default)]
    pub id: Option<String>,
    /// Origin.
    pub at: [f32; 2],
    /// Body relative to `at`.
    pub body: [f32; 4],
    /// Pins.
    pub pins: Vec<SchematicPin>,
}

impl SymbolEntry {
    /// The ref as a plain str.
    pub fn reference(&self) -> &str {
        self.reference.get_ref()
    }
}

/// `[schematic]`.
#[derive(Debug, Clone, PartialEq, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchematicSettings {
    /// Label boxes the router must avoid (absolute coordinates).
    #[serde(default)]
    pub keepouts: Vec<Keepout>,
}

/// A keepout with a note (replaces source comments).
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Keepout {
    /// `[x0, y0, x1, y1]`.
    pub rect: [f32; 4],
    /// Why it is there.
    #[serde(default)]
    pub note: Option<String>,
}

/// `[[probe]]`.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProbeEntry {
    /// Display name; unique in the file.
    pub name: Spanned<String>,
    /// Signal, for example `i(R1)` or `v(led_a)` (RFC 0004 grammar).
    pub signal: Spanned<String>,
}

impl ProbeEntry {
    /// The name as a plain str.
    pub fn name(&self) -> &str {
        self.name.get_ref()
    }

    /// The signal as a plain str.
    pub fn signal(&self) -> &str {
        self.signal.get_ref()
    }
}

/// Instrument kinds format 1 knows (RFC 0004).
pub const INSTRUMENT_KINDS: &[&str] = &["multimeter", "scope", "logic", "logger", "uart"];

/// Multimeter modes (DC only; AC and resistance come in a later addendum).
pub const MULTIMETER_MODES: &[&str] = &["dc_v", "dc_a"];

/// Multimeter current inputs (`params.input`, real-world profiles; default `mA`).
pub const MULTIMETER_INPUTS: &[&str] = &["uA", "mA", "A"];

/// The profile that reads exactly what the solver has and never loads the circuit.
pub const IDEAL_PROFILE: &str = "ideal";

/// Scope `params` keys.
pub const SCOPE_PARAMS: &[&str] = &["time_div", "trigger", "level"];

/// Scope trigger edges.
pub const SCOPE_TRIGGERS: &[&str] = &["rise", "fall"];

/// One `[[instrument]]` (RFC 0004, "`[[instrument]]` table").
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstrumentEntry {
    /// Ref, `X` prefix by convention; shares the ref namespace, never in the BOM.
    #[serde(rename = "ref")]
    pub reference: Spanned<String>,
    /// One of [`INSTRUMENT_KINDS`].
    pub kind: Spanned<String>,
    /// [`IDEAL_PROFILE`] (the default) or a `parts/instruments/<vendor>.<model>` id.
    #[serde(default)]
    pub profile: Option<Spanned<String>>,
    /// Multimeter only: one of [`MULTIMETER_MODES`].
    #[serde(default)]
    pub mode: Option<Spanned<String>>,
    /// `[[probe]]` names, or inline signals such as `v(vdd)`.
    pub probes: Vec<Spanned<String>>,
    /// Schematic placement of the instrument symbol.
    #[serde(default)]
    pub at: Option<[f32; 2]>,
    /// Kind-specific settings (scope: [`SCOPE_PARAMS`]).
    #[serde(default)]
    pub params: BTreeMap<String, ParamValue>,
}

impl InstrumentEntry {
    /// The ref as a plain str.
    pub fn reference(&self) -> &str {
        self.reference.get_ref()
    }

    /// The kind as a plain str.
    pub fn kind(&self) -> &str {
        self.kind.get_ref()
    }

    /// The profile id, [`IDEAL_PROFILE`] when left out.
    pub fn profile(&self) -> &str {
        self.profile.as_ref().map_or(IDEAL_PROFILE, |p| p.get_ref().as_str())
    }

    /// The multimeter mode, if set.
    pub fn mode(&self) -> Option<&str> {
        self.mode.as_ref().map(|m| m.get_ref().as_str())
    }
}
