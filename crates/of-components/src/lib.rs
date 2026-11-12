//! Built-in component models implemented in Rust.

use of_component_api::{Component, ComponentInfo};

/// Ideal resistor (stub).
#[derive(Debug, Clone)]
pub struct Resistor {
    info: ComponentInfo,
    /// Resistance in ohms.
    pub ohms: f64,
}

impl Resistor {
    /// New resistor.
    pub fn new(ohms: f64) -> Self {
        let info = ComponentInfo {
            part_id: "generic.resistor".into(),
            pins: vec!["1".into(), "2".into()],
        };
        Self { info, ohms }
    }
}

impl Component for Resistor {
    fn describe(&self) -> &ComponentInfo {
        &self.info
    }
}
