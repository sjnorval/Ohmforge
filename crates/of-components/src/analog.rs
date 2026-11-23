//! Bridge from placed library parts to analog solver devices.
//!
//! A [`Board`] is a list of [`Placement`]s: a part id, a reference
//! designator, the net each pin connects to and per-placement parameter
//! overrides (for example a resistor's `resistance`). [`build`] turns it
//! into an [`of_analog::Circuit`] without going through a SPICE deck:
//!
//! * primitive kinds (`resistor`, `capacitor`, `inductor`, `diode`, `led`,
//!   `npn`, `pnp`, `load`) become one device named after the reference
//!   designator (`R1` -> `r1`), with the part's `[analog]` pin order;
//! * a `switch` (push button) becomes a resistor of `ron` (closed) or
//!   `roff` (open), by the placement parameter `closed` (default open);
//!   [`BuiltBoard::switch_param`] gives the run-time change that presses
//!   or releases it;
//! * a `potentiometer` becomes two resistors split at the wiper, `<name>_a`
//!   (first pin to wiper, `position · resistance`) and `<name>_b` (wiper
//!   to last pin, the rest), by the placement parameter `position` (0 to
//!   1, default 0.5; each leg at least 1 mΩ); [`BuiltBoard::pot_params`]
//!   gives the run-time changes that turn it;
//! * `subckt` parts parse their SPICE library and are flattened as instance
//!   `x<refdes>` (`U1` -> `xu1`, internal nodes `xu1.*`).
//!
//! Parameters resolve as the part's value at the board's [`Corner`]; a
//! placement override replaces the typical value and keeps the part's
//! relative spread (a 330 Ω placement of a 1 % resistor is 326.7 Ω at
//! [`Corner::Min`]); the relative tolerance is stored on R, C and L for
//! Monte Carlo runs. Net names are case-insensitive; `0` and `gnd` are
//! ground. Pins without an analog role (for example I2C pins) may be left
//! unconnected.
//!
//! The circuit is returned unfinalised, so stimulus (supplies, pin drivers)
//! can be added before an analysis or an [`of_analog::AnalogEngine`] runs.

use std::collections::BTreeMap;
use std::fmt::Write as _;
use std::path::{Path, PathBuf};

use of_analog::devices::{Bjt, BjtModel, Capacitor, Diode, DiodeModel, Inductor, Resistor};
use of_analog::netlist::{BuildEnv, BuildError, add_deck};
use of_analog::{Circuit, CircuitError, SetParam};
use of_parts::{AnalogModel, Corner, PartFile};

/// A part plus the text of its SPICE library (for `subckt` parts).
#[derive(Debug, Clone)]
pub struct LibraryPart {
    /// The parsed `part.toml`.
    pub part: PartFile,
    /// Contents of the `[analog] file`, if any.
    pub spice: Option<String>,
}

/// Parts by id.
#[derive(Debug, Clone, Default)]
pub struct Library {
    parts: BTreeMap<String, LibraryPart>,
}

impl Library {
    /// An empty library.
    pub fn new() -> Self {
        Self::default()
    }

    /// Load every `part.toml` below `dir`, with their SPICE files.
    pub fn load_dir(dir: &Path) -> Result<Self, BridgeError> {
        let mut lib = Self::new();
        for (d, part) in of_parts::load_dir(dir).map_err(|e| BridgeError::Library(e.to_string()))? {
            lib.add_from_dir(part, &d)?;
        }
        Ok(lib)
    }

    /// Add a part whose SPICE file (if any) lives in `dir`.
    pub fn add_from_dir(&mut self, part: PartFile, dir: &Path) -> Result<(), BridgeError> {
        let spice = match part.analog.as_ref().and_then(|a| a.file.as_ref()) {
            Some(f) => {
                let path: PathBuf = dir.join(f);
                Some(
                    std::fs::read_to_string(&path)
                        .map_err(|e| BridgeError::Library(format!("{}: {e}", path.display())))?,
                )
            }
            None => None,
        };
        self.add(part, spice)
    }

    /// Add a part with its SPICE library text.
    pub fn add(&mut self, part: PartFile, spice: Option<String>) -> Result<(), BridgeError> {
        let problems = part.check();
        if !problems.is_empty() {
            return Err(BridgeError::Library(format!(
                "{}: {}",
                part.part.id,
                problems.join("; ")
            )));
        }
        self.parts.insert(part.part.id.clone(), LibraryPart { part, spice });
        Ok(())
    }

    /// A part by id.
    pub fn get(&self, id: &str) -> Option<&LibraryPart> {
        self.parts.get(id)
    }

    /// Part ids, sorted.
    pub fn ids(&self) -> impl Iterator<Item = &str> {
        self.parts.keys().map(String::as_str)
    }
}

/// One placed part.
#[derive(Debug, Clone, PartialEq)]
pub struct Placement {
    /// Reference designator, e.g. `R1` or `U1`.
    pub refdes: String,
    /// Library part id.
    pub part: String,
    /// Pin (number or name) to net name.
    pub nets: BTreeMap<String, String>,
    /// Parameter overrides, e.g. `resistance = 330`.
    pub params: BTreeMap<String, f64>,
}

impl Placement {
    /// Place `part` as `refdes` with `(pin, net)` connections.
    pub fn new(refdes: &str, part: &str, nets: &[(&str, &str)]) -> Self {
        Self {
            refdes: refdes.to_owned(),
            part: part.to_owned(),
            nets: nets.iter().map(|(p, n)| ((*p).to_owned(), (*n).to_owned())).collect(),
            params: BTreeMap::new(),
        }
    }

    /// Override a parameter.
    pub fn with(mut self, key: &str, value: f64) -> Self {
        self.params.insert(key.to_ascii_lowercase(), value);
        self
    }
}

/// A set of placements plus board-wide settings.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Board {
    /// Placed parts.
    pub placements: Vec<Placement>,
    /// Which value of toleranced parameters to use.
    pub corner: Corner,
    /// Temperatures for tempcos and device models.
    pub env: Env,
}

/// Board temperatures (°C).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Env {
    /// Ambient (circuit) temperature.
    pub temp_c: f64,
    /// Nominal temperature of the model parameters.
    pub tnom_c: f64,
}

impl Default for Env {
    fn default() -> Self {
        Self {
            temp_c: 27.0,
            tnom_c: 27.0,
        }
    }
}

/// Bridge errors.
#[derive(Debug, Clone, PartialEq, thiserror::Error)]
pub enum BridgeError {
    /// A part file or library problem.
    #[error("parts library: {0}")]
    Library(String),
    /// A placement refers to an unknown part.
    #[error("{0}: unknown part '{1}'")]
    UnknownPart(String, String),
    /// The part has no analog model.
    #[error("{0}: part '{1}' has no [analog] model")]
    NoAnalogModel(String, String),
    /// A pin needed by the analog model is not connected.
    #[error("{0}: pin '{1}' is not connected")]
    Unconnected(String, String),
    /// A placement connects a pin the part does not have.
    #[error("{0}: part '{1}' has no pin '{2}'")]
    UnknownPin(String, String, String),
    /// A required parameter is missing or invalid.
    #[error("{0}: {1}")]
    Param(String, String),
    /// Two placements share a reference designator.
    #[error("duplicate reference designator '{0}'")]
    Duplicate(String),
    /// The SPICE subcircuit did not parse or build.
    #[error("{0}: {1}")]
    Spice(String, String),
    /// Circuit-level problem.
    #[error(transparent)]
    Circuit(#[from] CircuitError),
}

/// A board turned into a circuit.
#[derive(Debug)]
pub struct BuiltBoard {
    /// The circuit (not yet finalised).
    pub circuit: Circuit,
    /// Device name (or subcircuit instance prefix) of each refdes.
    pub instances: BTreeMap<String, String>,
    /// Ignored parameters and similar notes.
    pub warnings: Vec<String>,
    /// Switches by refdes.
    pub switches: BTreeMap<String, Switch>,
    /// Potentiometers by refdes.
    pub pots: BTreeMap<String, Pot>,
}

/// A placed `potentiometer`: the two resistors that stand for its track
/// and the track's resistance.
#[derive(Debug, Clone, PartialEq)]
pub struct Pot {
    /// Device name of the first-pin-to-wiper resistor.
    pub a: String,
    /// Device name of the wiper-to-last-pin resistor.
    pub b: String,
    /// End-to-end resistance (Ω).
    pub resistance: f64,
}

/// Smallest leg of a potentiometer (Ω), so a wiper at an end stays a
/// finite conductance.
pub const POT_MIN_LEG: f64 = 1e-3;

impl Pot {
    /// The two leg resistances with the wiper at `position` (clamped to
    /// 0..=1).
    pub fn legs(&self, position: f64) -> (f64, f64) {
        let x = if position.is_nan() {
            0.5
        } else {
            position.clamp(0.0, 1.0)
        };
        (
            (x * self.resistance).max(POT_MIN_LEG),
            ((1.0 - x) * self.resistance).max(POT_MIN_LEG),
        )
    }
}

/// A placed `switch` part: the resistor that stands for it and its two
/// resistances.
#[derive(Debug, Clone, PartialEq)]
pub struct Switch {
    /// Device name of the resistor.
    pub device: String,
    /// Closed resistance (Ω).
    pub ron: f64,
    /// Open resistance (Ω).
    pub roff: f64,
}

impl BuiltBoard {
    /// The run-time change that closes (`true`) or opens switch `refdes`,
    /// for [`of_analog::AnalogEngine::on_code`]; `None` if it is not a
    /// switch.
    pub fn switch_param(&self, refdes: &str, closed: bool) -> Option<SetParam> {
        let s = self.switches.get(refdes)?;
        Some(SetParam {
            device: s.device.clone(),
            param: "r".into(),
            value: if closed { s.ron } else { s.roff },
        })
    }

    /// The run-time changes that put potentiometer `refdes`'s wiper at
    /// `position` (0 = at its first pin, 1 = at its last), for
    /// [`of_analog::AnalogEngine::on_code`]; `None` if it is not a
    /// potentiometer.
    pub fn pot_params(&self, refdes: &str, position: f64) -> Option<Vec<SetParam>> {
        let p = self.pots.get(refdes)?;
        let (a, b) = p.legs(position);
        let set = |device: &str, value| SetParam {
            device: device.to_owned(),
            param: "r".into(),
            value,
        };
        Some(vec![set(&p.a, a), set(&p.b, b)])
    }
}

/// Parameters of one placement, resolved at the board's corner.
struct Resolved<'a> {
    refdes: &'a str,
    model: &'a AnalogModel,
    overrides: &'a BTreeMap<String, f64>,
    corner: Corner,
}

impl Resolved<'_> {
    /// The override (moved to the corner by the part's relative spread),
    /// else the part's value at the corner.
    fn get(&self, key: &str) -> Option<f64> {
        let spec = self.model.params.get(key).map(|p| p.spec());
        match (self.overrides.get(key), spec) {
            (Some(&v), Some(s)) => match (s.at(self.corner), s.typical()) {
                (Some(c), Some(t)) if t != 0.0 => Some(v * c / t),
                _ => Some(v),
            },
            (Some(&v), None) => Some(v),
            (None, s) => s?.at(self.corner),
        }
    }
    fn need(&self, key: &str) -> Result<f64, BridgeError> {
        self.get(key)
            .filter(|v| v.is_finite())
            .ok_or_else(|| BridgeError::Param(self.refdes.to_owned(), format!("missing parameter '{key}'")))
    }
    /// Tolerance of the principal value: none if overridden without one.
    fn tolerance(&self, key: &str) -> Option<f64> {
        self.overrides
            .get(&format!("{key}_tol"))
            .copied()
            .or_else(|| self.model.params.get(key).and_then(|p| p.spec().tolerance()))
    }
    /// All numeric parameters except `skip` (device model cards).
    fn all(&self, skip: &[&str]) -> BTreeMap<String, f64> {
        let keys: std::collections::BTreeSet<&String> = self.model.params.keys().chain(self.overrides.keys()).collect();
        let mut m: BTreeMap<String, f64> = keys
            .into_iter()
            .filter_map(|k| Some((k.clone(), self.get(k)?)))
            .collect();
        m.retain(|k, _| !skip.contains(&k.as_str()) && !k.ends_with("_tol"));
        m
    }
}

/// Net names of the model's pins, in terminal order.
fn pin_nets(pl: &Placement, part: &PartFile, model: &AnalogModel) -> Result<Vec<String>, BridgeError> {
    for key in pl.nets.keys() {
        if part.pin(key).is_none() {
            return Err(BridgeError::UnknownPin(
                pl.refdes.clone(),
                part.part.id.clone(),
                key.clone(),
            ));
        }
    }
    model
        .pins
        .iter()
        .map(|mp| {
            let pin = part
                .pin(mp)
                .ok_or_else(|| BridgeError::UnknownPin(pl.refdes.clone(), part.part.id.clone(), mp.clone()))?;
            pl.nets
                .iter()
                .find(|(k, _)| part.pin(k).is_some_and(|p| p.number == pin.number))
                .map(|(_, n)| n.to_ascii_lowercase())
                .ok_or_else(|| BridgeError::Unconnected(pl.refdes.clone(), format!("{} ({})", pin.name, pin.number)))
        })
        .collect()
}

const KELVIN: f64 = 273.15;

/// Result of adding one placement to a circuit.
#[derive(Debug)]
pub struct PlacementAdded {
    /// Device name (or subcircuit instance prefix).
    pub instance: String,
    /// Switch metadata, if this placement is a switch.
    pub switch: Option<Switch>,
    /// Potentiometer metadata, if this placement is a potentiometer.
    pub pot: Option<Pot>,
    /// Ignored parameters and similar notes.
    pub warnings: Vec<String>,
}

/// Add one placement's devices to an existing (unfinalised) circuit.
pub fn add_placement(
    c: &mut Circuit,
    lib: &Library,
    pl: &Placement,
    corner: Corner,
    env: &Env,
) -> Result<PlacementAdded, BridgeError> {
    let build_env = BuildEnv {
        temp_c: env.temp_c,
        tnom_c: env.tnom_c,
    };
    let temp_k = env.temp_c + KELVIN;
    let dt = env.temp_c - env.tnom_c;
    let lp = lib
        .get(&pl.part)
        .ok_or_else(|| BridgeError::UnknownPart(pl.refdes.clone(), pl.part.clone()))?;
    let model = lp
        .part
        .analog
        .as_ref()
        .ok_or_else(|| BridgeError::NoAnalogModel(pl.refdes.clone(), pl.part.clone()))?;
    let nets = pin_nets(pl, &lp.part, model)?;
    let r = Resolved {
        refdes: &pl.refdes,
        model,
        overrides: &pl.params,
        corner,
    };
    let name = pl.refdes.to_ascii_lowercase();
    let nodes: Vec<usize> = nets.iter().map(|n| c.node(n)).collect();
    let tc = 1.0 + r.get("tc1").unwrap_or(0.0) * dt + r.get("tc2").unwrap_or(0.0) * dt * dt;
    let with_tnom = |mut m: BTreeMap<String, f64>| {
        m.entry("tnom".into()).or_insert(env.tnom_c);
        m
    };
    let mut warnings = Vec::new();
    let note = |w: &mut Vec<String>, unknown: Vec<String>| {
        if !unknown.is_empty() {
            w.push(format!(
                "{}: model parameters not supported, ignored: {}",
                pl.refdes,
                unknown.join(", ")
            ));
        }
    };
    let positive = |key: &str| -> Result<f64, BridgeError> {
        let v = r.need(key)?;
        if v > 0.0 {
            Ok(v)
        } else {
            Err(BridgeError::Param(
                pl.refdes.clone(),
                format!("'{key}' must be positive (got {v})"),
            ))
        }
    };
    let mut switch = None;
    let mut pot = None;
    let instance = match model.kind.as_str() {
        "resistor" => {
            let ohms = positive("resistance")? * tc;
            c.add(Resistor::new(&name, nodes[0], nodes[1], ohms).with_tolerance(r.tolerance("resistance")))?;
            name
        }
        "load" => {
            let ohms = match r.get("resistance") {
                Some(v) => v,
                None => positive("voltage")? / positive("current")?,
            };
            c.add(Resistor::new(&name, nodes[0], nodes[1], ohms))?;
            name
        }
        "switch" => {
            let s = Switch {
                device: name.clone(),
                ron: positive("ron")?,
                roff: positive("roff")?,
            };
            let ohms = if switch_closed(&pl.params) { s.ron } else { s.roff };
            c.add(Resistor::new(&name, nodes[0], nodes[1], ohms))?;
            switch = Some(s);
            name
        }
        "potentiometer" => {
            let p = Pot {
                a: format!("{name}_a"),
                b: format!("{name}_b"),
                resistance: positive("resistance")? * tc,
            };
            let (ra, rb) = p.legs(pot_position(&pl.params));
            let tol = r.tolerance("resistance");
            c.add(Resistor::new(&p.a, nodes[0], nodes[1], ra).with_tolerance(tol))?;
            c.add(Resistor::new(&p.b, nodes[1], nodes[2], rb).with_tolerance(tol))?;
            pot = Some(p);
            name
        }
        "capacitor" => {
            let cap = positive("capacitance")? * tc;
            c.add(
                Capacitor::new(&name, nodes[0], nodes[1], cap, r.get("ic"))
                    .with_parasitics(r.get("esr").unwrap_or(0.0), r.get("esl").unwrap_or(0.0))
                    .with_tolerance(r.tolerance("capacitance")),
            )?;
            name
        }
        "inductor" => {
            let l = positive("inductance")? * tc;
            c.add(
                Inductor::new(&name, nodes[0], nodes[1], l, r.get("ic"))
                    .with_series_resistance(r.get("rser").unwrap_or(0.0))
                    .with_tolerance(r.tolerance("inductance")),
            )?;
            name
        }
        "diode" | "led" => {
            let p = with_tnom(r.all(&["area"]));
            let (dm, unknown) = if model.kind == "led" {
                DiodeModel::led_from_params(&p)
            } else {
                DiodeModel::from_params(&p)
            };
            note(&mut warnings, unknown);
            let area = r.get("area").unwrap_or(1.0);
            c.add(Diode::new(&name, nodes[0], nodes[1], dm, area, false).with_temp(temp_k))?;
            name
        }
        "npn" | "pnp" => {
            let pol = if model.kind == "npn" { 1.0 } else { -1.0 };
            let (bm, unknown) = BjtModel::from_params(pol, &with_tnom(r.all(&["area"])));
            note(&mut warnings, unknown);
            let area = r.get("area").unwrap_or(1.0);
            c.add(Bjt::new(&name, nodes[0], nodes[1], nodes[2], bm, area, false).with_temp(temp_k))?;
            name
        }
        "subckt" => {
            let text = lp
                .spice
                .as_deref()
                .ok_or_else(|| BridgeError::Library(format!("{}: SPICE file not loaded", pl.part)))?;
            let sub = model.subckt.as_deref().unwrap_or_default();
            let inst = format!("x{name}");
            let mut card = format!("{inst} {} {sub}", nets.join(" "));
            for (k, v) in r.all(&[]) {
                let _ = write!(card, " {k}={v:e}");
            }
            let deck_text = format!("* {} ({})\n{text}\n{card}\n.end\n", pl.refdes, pl.part);
            let deck = of_netlist::spice::parse_str(&deck_text)
                .map_err(|e| BridgeError::Spice(pl.refdes.clone(), e.to_string()))?;
            warnings.extend(deck.warnings.iter().map(|w| format!("{}: {w}", pl.refdes)));
            add_deck(c, &deck, &build_env, &mut warnings).map_err(|e| match e {
                BuildError::Circuit(ce) => BridgeError::Circuit(ce),
                other => BridgeError::Spice(pl.refdes.clone(), other.to_string()),
            })?;
            inst
        }
        other => {
            return Err(BridgeError::Library(format!(
                "{}: unknown analog kind '{other}'",
                pl.part
            )));
        }
    };
    Ok(PlacementAdded {
        instance,
        switch,
        pot,
        warnings,
    })
}

/// Build the circuit for `board`.
pub fn build(lib: &Library, board: &Board) -> Result<BuiltBoard, BridgeError> {
    let mut c = Circuit::new();
    let mut instances = BTreeMap::new();
    let mut warnings = Vec::new();
    let mut switches = BTreeMap::new();
    let mut pots = BTreeMap::new();
    for pl in &board.placements {
        if instances.contains_key(&pl.refdes) {
            return Err(BridgeError::Duplicate(pl.refdes.clone()));
        }
        let added = add_placement(&mut c, lib, pl, board.corner, &board.env)?;
        warnings.extend(added.warnings);
        if let Some(s) = added.switch {
            switches.insert(pl.refdes.clone(), s);
        }
        if let Some(p) = added.pot {
            pots.insert(pl.refdes.clone(), p);
        }
        instances.insert(pl.refdes.clone(), added.instance);
    }
    Ok(BuiltBoard {
        circuit: c,
        instances,
        warnings,
        switches,
        pots,
    })
}

/// A potentiometer placement's wiper `position` (default 0.5).
fn pot_position(params: &BTreeMap<String, f64>) -> f64 {
    params.get("position").copied().unwrap_or(0.5)
}

/// A switch placement's `closed` parameter (non-zero is closed).
fn switch_closed(params: &BTreeMap<String, f64>) -> bool {
    params.get("closed").is_some_and(|&v| v != 0.0)
}

/// The board as SPICE cards (for inspection or an external reference
/// run): one card per primitive, `.model` cards, and each subcircuit's
/// library once. Values use the board's corner; LED cards use the Ohmforge
/// `led` model type and capacitor ESR/ESL use `esr=`/`esl=`.
pub fn to_spice(lib: &Library, board: &Board) -> Result<String, BridgeError> {
    let mut cards = String::new();
    let mut models = String::new();
    let mut libs: BTreeMap<String, String> = BTreeMap::new();
    for pl in &board.placements {
        let lp = lib
            .get(&pl.part)
            .ok_or_else(|| BridgeError::UnknownPart(pl.refdes.clone(), pl.part.clone()))?;
        let model = lp
            .part
            .analog
            .as_ref()
            .ok_or_else(|| BridgeError::NoAnalogModel(pl.refdes.clone(), pl.part.clone()))?;
        let nets = pin_nets(pl, &lp.part, model)?.join(" ");
        let r = Resolved {
            refdes: &pl.refdes,
            model,
            overrides: &pl.params,
            corner: board.corner,
        };
        let name = pl.refdes.to_ascii_lowercase();
        let kv = |m: BTreeMap<String, f64>| {
            m.iter()
                .map(|(k, v)| format!("{k}={v:e}"))
                .collect::<Vec<_>>()
                .join(" ")
        };
        let _ = match model.kind.as_str() {
            "resistor" => writeln!(cards, "R{name} {nets} {:e}", r.need("resistance")?),
            "load" => {
                let ohms = match r.get("resistance") {
                    Some(v) => v,
                    None => r.need("voltage")? / r.need("current")?,
                };
                writeln!(cards, "R{name} {nets} {ohms:e}")
            }
            "switch" => {
                let ohms = r.need(if switch_closed(&pl.params) { "ron" } else { "roff" })?;
                writeln!(cards, "R{name} {nets} {ohms:e}")
            }
            "potentiometer" => {
                let mut parts = nets.split(' ');
                let (n1, nw, n3) = (
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                    parts.next().unwrap_or_default(),
                );
                let p = Pot {
                    a: String::new(),
                    b: String::new(),
                    resistance: r.need("resistance")?,
                };
                let (ra, rb) = p.legs(pot_position(&pl.params));
                writeln!(cards, "R{name}_a {n1} {nw} {ra:e}\nR{name}_b {nw} {n3} {rb:e}")
            }
            "capacitor" => {
                let extra = kv(["esr", "esl"]
                    .iter()
                    .filter_map(|k| Some(((*k).to_owned(), r.get(k).filter(|v| *v != 0.0)?)))
                    .collect());
                writeln!(cards, "C{name} {nets} {:e} {extra}", r.need("capacitance")?)
            }
            "inductor" => writeln!(cards, "L{name} {nets} {:e}", r.need("inductance")?),
            "diode" | "led" => {
                let _ = writeln!(
                    models,
                    ".model m{name} {}({})",
                    if model.kind == "led" { "led" } else { "d" },
                    kv(r.all(&["area"]))
                );
                writeln!(cards, "D{name} {nets} m{name}")
            }
            "npn" | "pnp" => {
                let _ = writeln!(models, ".model m{name} {}({})", model.kind, kv(r.all(&["area"])));
                writeln!(cards, "Q{name} {nets} m{name}")
            }
            "subckt" => {
                if let Some(t) = &lp.spice {
                    libs.insert(pl.part.clone(), t.clone());
                }
                let sub = model.subckt.as_deref().unwrap_or_default();
                writeln!(cards, "X{name} {nets} {sub} {}", kv(r.all(&[])))
            }
            other => {
                return Err(BridgeError::Library(format!(
                    "{}: unknown analog kind '{other}'",
                    pl.part
                )));
            }
        };
    }
    let mut out = String::from("* Generated by the Ohmforge parts bridge\n");
    for t in libs.values() {
        out.push_str(t);
        out.push('\n');
    }
    out.push_str(&models);
    out.push_str(&cards);
    Ok(out)
}
