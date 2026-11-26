//! Built-in component models implemented in Rust, and the bridge from
//! placed parts to the analog solver ([`analog`]).

pub mod analog;
pub mod bounce;
pub mod library_component;
pub mod psu;

pub use bounce::BounceSettings;
pub use library_component::LibraryComponent;
pub use of_parts::Corner;
pub use psu::{BenchPsu, SupplyMode, SupplySettings};

use std::sync::Arc;

use of_engine::{ComponentSpec, Registry};

/// Models built from a parts-library part through the analog bridge.
pub const LIBRARY_MODELS: &[&str] = &[
    "rust:resistor",
    "rust:capacitor",
    "rust:led",
    "rust:diode",
    "rust:bjt",
    "rust:potentiometer",
    "rust:switch",
];

/// Register the built-in models: every [`LIBRARY_MODELS`] entry and every
/// `spice:<file>` model a `subckt` part in `lib` names (the bridge flattens
/// its SPICE library) as a [`LibraryComponent`] from `lib`, and
/// `rust:bench-psu` as a [`BenchPsu`].
pub fn register_builtin(reg: &mut Registry, lib: Arc<Library>, corner: Corner, env: Env) {
    let spice: std::collections::BTreeSet<String> = lib
        .ids()
        .filter_map(|id| lib.get(id))
        .map(|p| p.part.part.model.clone())
        .filter(|m| m.starts_with("spice:"))
        .collect();
    let models: Vec<String> = LIBRARY_MODELS.iter().map(|m| (*m).to_owned()).chain(spice).collect();
    for model in &models {
        let lib = Arc::clone(&lib);
        reg.register(
            model,
            Box::new(move |spec: ComponentSpec<'_>| {
                let c = LibraryComponent::new(
                    spec.refdes,
                    spec.part_id,
                    Arc::clone(&lib),
                    spec.numeric_params(),
                    corner,
                    env,
                )?;
                Ok(Box::new(c) as Box<dyn Component>)
            }),
        );
    }
    reg.register(
        "rust:bench-psu",
        Box::new(|spec: ComponentSpec<'_>| {
            let mut s = SupplySettings::default();
            let p = spec.numeric_params();
            if let Some(&v) = p.get("volts") {
                s.volts = v;
            }
            if let Some(&v) = p.get("series_r") {
                s.series_r = v;
            }
            if let Some(&v) = p.get("current_limit") {
                s.current_limit = v;
            }
            if let Some(&v) = p.get("on") {
                s.on = v != 0.0;
            }
            if let (Some(&lo), Some(&hi)) = (p.get("rated_min"), p.get("rated_max")) {
                s.rated = Some((lo, hi));
            }
            Ok(Box::new(BenchPsu::new(spec.refdes, s)) as Box<dyn Component>)
        }),
    );
}

use analog::{Env, Library};
use of_component_api::{Component, ComponentInfo, ElabCtx, ElabError, PinKind, PinSpec, SolutionView};

/// Ideal resistor component (direct device, no library lookup).
#[derive(Debug, Clone)]
pub struct Resistor {
    info: ComponentInfo,
    /// Resistance in ohms.
    pub ohms: f64,
    pin_nets: [String; 2],
}

impl Resistor {
    /// New resistor with reference designator `refdes`.
    pub fn new(refdes: &str, ohms: f64) -> Self {
        Self {
            info: ComponentInfo {
                part_id: "generic.resistor".into(),
                refdes: refdes.to_owned(),
                pins: vec![
                    PinSpec {
                        name: "1".into(),
                        kind: PinKind::Passive,
                    },
                    PinSpec {
                        name: "2".into(),
                        kind: PinKind::Passive,
                    },
                ],
            },
            ohms,
            pin_nets: [String::new(), String::new()],
        }
    }
}

impl Component for Resistor {
    fn info(&self) -> &ComponentInfo {
        &self.info
    }

    fn elaborate(&mut self, cx: &mut dyn ElabCtx) -> Result<(), ElabError> {
        let a = cx.pin_node("1")?;
        let b = cx.pin_node("2")?;
        self.pin_nets[0] = net_name(cx.circuit_mut(), a);
        self.pin_nets[1] = net_name(cx.circuit_mut(), b);
        let ohms = cx.param("resistance").unwrap_or(self.ohms);
        self.ohms = ohms;
        let name = self.info.refdes.to_ascii_lowercase();
        cx.add_device(Box::new(of_analog::devices::Resistor::new(&name, a, b, ohms)))?;
        Ok(())
    }

    fn terminal_currents(&self, view: &dyn SolutionView, out: &mut [f64]) {
        out.fill(0.0);
        if out.len() < 2 || self.ohms == 0.0 {
            return;
        }
        let v0 = view.value(&format!("v({})", self.pin_nets[0])).unwrap_or(0.0);
        let v1 = view.value(&format!("v({})", self.pin_nets[1])).unwrap_or(0.0);
        let i = (v0 - v1) / self.ohms;
        out[0] = i;
        out[1] = -i;
    }
}

fn net_name(circuit: &of_analog::Circuit, node: of_analog::Var) -> String {
    circuit
        .vars()
        .get(node)
        .map(|v| {
            v.name
                .strip_prefix("v(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("gnd")
                .to_owned()
        })
        .unwrap_or_else(|| "gnd".into())
}
