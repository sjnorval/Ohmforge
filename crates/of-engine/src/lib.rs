//! Base engine: elaborates [`Component`]s into an [`of_analog::Circuit`]
//! and checks design invariants (RFC 0001 / engine-components §4).

use std::collections::BTreeMap;

use of_analog::{Circuit, Device, Var, analysis, results::OpPoint};
use of_component_api::{Component, DeviceRef, ElabCtx, ElabError, PinKind, SolutionView};

/// Engine build / invariant errors.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EngineError {
    /// Elaboration of a component failed.
    #[error("{0}")]
    Elab(#[from] ElabError),
    /// An invariant failed.
    #[error("invariant: {0}")]
    Invariant(String),
}

/// A net in the circuit under construction.
#[derive(Debug, Clone)]
struct NetInfo {
    /// True when this net is the single ground reference.
    ground: bool,
    /// Components (refdes, pin) attached to this net.
    pins: Vec<(String, String)>,
}

/// Context passed to one component during elaboration.
struct ElabState<'a> {
    circuit: &'a mut Circuit,
    pin_nets: BTreeMap<String, String>,
    params: BTreeMap<String, f64>,
    seed: u64,
    refdes: String,
    device_names: &'a mut Vec<String>,
}

impl ElabCtx for ElabState<'_> {
    fn pin_node(&self, pin: &str) -> Result<Var, ElabError> {
        let net = self
            .pin_nets
            .get(pin)
            .or_else(|| {
                self.pin_nets
                    .iter()
                    .find(|(k, _)| k.eq_ignore_ascii_case(pin))
                    .map(|(_, v)| v)
            })
            .ok_or_else(|| ElabError::msg(format!("{}: pin '{pin}' is not connected", self.refdes)))?;
        self.circuit
            .find_node(net)
            .ok_or_else(|| ElabError::msg(format!("{}: unknown net '{net}'", self.refdes)))
    }

    fn internal_node(&mut self, name: &str) -> Var {
        let full = format!("{}.{}", self.refdes.to_ascii_lowercase(), name.to_ascii_lowercase());
        self.circuit.node(&full)
    }

    fn add_device(&mut self, dev: Box<dyn Device>) -> Result<DeviceRef, ElabError> {
        let name = dev.name().to_owned();
        self.circuit.add_boxed(dev).map_err(ElabError::msg)?;
        let idx = self.device_names.len() as u32;
        self.device_names.push(name);
        Ok(DeviceRef(idx))
    }

    fn circuit_mut(&mut self) -> &mut Circuit {
        self.circuit
    }

    fn param(&self, name: &str) -> Option<f64> {
        self.params.get(&name.to_ascii_lowercase()).copied()
    }

    fn seed(&self) -> u64 {
        self.seed
    }

    fn refdes(&self) -> &str {
        &self.refdes
    }
}

/// Operating-point wrapper for [`SolutionView`].
pub struct OpView<'a>(pub &'a OpPoint);

impl SolutionView for OpView<'_> {
    fn value(&self, name: &str) -> Option<f64> {
        self.0.get(name)
    }
}

struct Pending {
    component: Box<dyn Component>,
    pin_nets: BTreeMap<String, String>,
    params: BTreeMap<String, f64>,
    seed: u64,
}

/// Builder that owns nets and components until [`EngineBuilder::elaborate`].
pub struct EngineBuilder {
    ground: String,
    nets: BTreeMap<String, NetInfo>,
    pending: Vec<Pending>,
}

impl EngineBuilder {
    /// Start a circuit with ground net `ground` (must be `gnd` in E2).
    pub fn new(ground: &str) -> Self {
        let key = ground.to_ascii_lowercase();
        let mut nets = BTreeMap::new();
        nets.insert(
            key.clone(),
            NetInfo {
                ground: true,
                pins: Vec::new(),
            },
        );
        Self {
            ground: key,
            nets,
            pending: Vec::new(),
        }
    }

    /// Declare a non-ground net (idempotent).
    pub fn net(&mut self, name: &str) -> &mut Self {
        let key = name.to_ascii_lowercase();
        if key != self.ground {
            self.nets.entry(key).or_insert_with(|| NetInfo {
                ground: false,
                pins: Vec::new(),
            });
        }
        self
    }

    /// Place a component. `pins` maps pin name → net name.
    pub fn add(
        &mut self,
        component: Box<dyn Component>,
        pins: &[(&str, &str)],
        params: &[(&str, f64)],
        seed: u64,
    ) -> Result<&mut Self, EngineError> {
        let info = component.info().clone();
        let mut pin_nets = BTreeMap::new();
        for (pin, net) in pins {
            let net_key = net.to_ascii_lowercase();
            if !self.nets.contains_key(&net_key) {
                return Err(EngineError::Invariant(format!(
                    "net '{net}' is not declared (add it with EngineBuilder::net)"
                )));
            }
            self.nets
                .get_mut(&net_key)
                .expect("net exists")
                .pins
                .push((info.refdes.clone(), (*pin).to_owned()));
            pin_nets.insert((*pin).to_owned(), net_key);
        }
        for p in &info.pins {
            if !pin_nets.keys().any(|k| k.eq_ignore_ascii_case(&p.name)) {
                return Err(EngineError::Invariant(format!(
                    "{}: pin '{}' is not connected",
                    info.refdes, p.name
                )));
            }
        }
        let params = params.iter().map(|(k, v)| (k.to_ascii_lowercase(), *v)).collect();
        self.pending.push(Pending {
            component,
            pin_nets,
            params,
            seed,
        });
        Ok(self)
    }

    /// Elaborate all components and check invariants 1–4.
    pub fn elaborate(mut self) -> Result<BuiltEngine, EngineError> {
        if self.ground != "gnd" && self.ground != "0" {
            return Err(EngineError::Invariant(
                "E2 requires the ground net to be named 'gnd' (solver node 0)".into(),
            ));
        }
        let ground_count = self.nets.values().filter(|n| n.ground).count();
        if ground_count != 1 {
            return Err(EngineError::Invariant(format!(
                "exactly one ground net required, found {ground_count}"
            )));
        }
        for (name, net) in &self.nets {
            if net.pins.is_empty() && !net.ground {
                return Err(EngineError::Invariant(format!("net '{name}' has no pins (floating)")));
            }
        }

        let mut circuit = Circuit::new();
        for name in self.nets.keys() {
            if name != "gnd" && name != "0" {
                let _ = circuit.node(name);
            }
        }

        let mut all_devices = Vec::new();
        let mut components = Vec::new();

        for mut p in self.pending.drain(..) {
            let info = p.component.info().clone();
            for pin in &info.pins {
                if matches!(pin.kind, PinKind::Power | PinKind::Ground)
                    && !p.pin_nets.keys().any(|k| k.eq_ignore_ascii_case(&pin.name))
                {
                    return Err(EngineError::Invariant(format!(
                        "{}: supply pin '{}' is not connected",
                        info.refdes, pin.name
                    )));
                }
            }

            let before_devs: Vec<String> = circuit.devices().map(|d| d.name().to_owned()).collect();
            let mut cx = ElabState {
                circuit: &mut circuit,
                pin_nets: p.pin_nets.clone(),
                params: p.params.clone(),
                seed: p.seed,
                refdes: info.refdes.clone(),
                device_names: &mut all_devices,
            };
            p.component.elaborate(&mut cx)?;
            let after_devs: Vec<String> = circuit.devices().map(|d| d.name().to_owned()).collect();
            let device_names: Vec<String> = after_devs
                .into_iter()
                .filter(|n| !before_devs.iter().any(|b| b == n))
                .collect();
            for n in &device_names {
                if !all_devices.iter().any(|e| e == n) {
                    all_devices.push(n.clone());
                }
            }
            components.push(PlacedComponent {
                component: p.component,
                pin_nets: p.pin_nets,
                device_names,
                params: p.params,
                seed: p.seed,
            });
        }

        Ok(BuiltEngine {
            circuit,
            components,
            device_names: all_devices,
            ground: self.ground,
        })
    }
}

/// A component after elaboration.
pub struct PlacedComponent {
    /// The live component model.
    pub component: Box<dyn Component>,
    /// Pin → net map.
    pub pin_nets: BTreeMap<String, String>,
    /// Device instance names this component added.
    pub device_names: Vec<String>,
    /// Placement parameters.
    pub params: BTreeMap<String, f64>,
    /// Seed.
    pub seed: u64,
}

/// Circuit and components after a successful elaborate.
pub struct BuiltEngine {
    /// Circuit ready for analysis (not yet finalised by the OP helper).
    pub circuit: Circuit,
    /// Placed components.
    pub components: Vec<PlacedComponent>,
    /// All device names in elaboration order.
    pub device_names: Vec<String>,
    /// Ground net name.
    pub ground: String,
}

impl BuiltEngine {
    /// Run a DC operating point.
    pub fn operating_point(&mut self) -> Result<OpPoint, String> {
        analysis::operating_point(&mut self.circuit, &analysis::Options::default()).map_err(|e| e.to_string())
    }

    /// Each component's terminal currents must sum to zero within `tol`.
    pub fn check_kcl(&self, op: &OpPoint, tol: f64) -> Result<(), EngineError> {
        let view = OpView(op);
        for pc in &self.components {
            let n = pc.component.info().pins.len();
            let mut i = vec![0.0; n];
            pc.component.terminal_currents(&view, &mut i);
            let sum: f64 = i.iter().sum();
            if !sum.is_finite() || sum.abs() > tol {
                return Err(EngineError::Invariant(format!(
                    "{}: terminal currents sum to {sum} (tol {tol})",
                    pc.component.info().refdes
                )));
            }
        }
        Ok(())
    }
}
