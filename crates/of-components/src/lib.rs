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
