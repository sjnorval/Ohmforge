//! Virtual voltmeters and ammeters on the live schematic. Placement and
//! readings are `of_instruments::meter` (E5c); this module keeps the
//! schematic-side names. The pre-E5c readout below is test-only: it is the
//! reference of the old-versus-new equality test.

#[cfg(test)]
use std::collections::BTreeMap;

use of_instruments::Multimeter;

/// A voltmeter or a non-disturbing ammeter.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeterKind {
    /// `V(A) − V(B)`.
    Volt,
    /// Current through the single part that joins A and B, from A toward B.
    Amp,
}

impl MeterKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Volt => "Voltmeter",
            Self::Amp => "Ammeter",
        }
    }
}

/// One end of a placed meter, named as the user clicked it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeRef {
    /// Stable click-target id (`"gpio8"`, `"led_k"`, `"wiper"`, …).
    pub id: String,
    /// Circuit node (`"gpio8"`, `"0"`, `"vdd"`, …).
    pub node: String,
    /// Short label for the readout list.
    pub label: String,
}

/// A meter the user has placed.
#[derive(Debug, Clone, PartialEq)]
pub struct PlacedMeter {
    pub id: u64,
    pub kind: MeterKind,
    pub a: ProbeRef,
    pub b: ProbeRef,
    /// What the worker samples and averages.
    pub meter: Multimeter,
}

/// Why an ammeter cannot be placed (or cannot be read).
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AmpError {
    SameNet,
    NoUniqueBranch,
}

/// A two-terminal part and the current through it, from `a` toward `b`.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub struct Branch {
    pub name: String,
    pub a: String,
    pub b: String,
    pub amps: f64,
}

/// Node voltages and two-terminal part currents, sampled from the solver
/// without inserting sense elements.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Default)]
pub struct CircuitReadout {
    /// Node voltages, including `"0"` / `"gnd"` at 0 V when present.
    pub nodes: BTreeMap<String, f64>,
    pub branches: Vec<Branch>,
}

#[cfg(test)]
impl CircuitReadout {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_node(&mut self, name: &str, volts: f64) {
        self.nodes.insert(name.to_ascii_lowercase(), volts);
    }

    /// Record a part between `a` and `b` carrying `amps` from `a` toward `b`.
    pub fn push_branch(&mut self, name: &str, a: &str, b: &str, amps: f64) {
        self.branches.push(Branch {
            name: name.to_owned(),
            a: a.to_ascii_lowercase(),
            b: b.to_ascii_lowercase(),
            amps,
        });
    }

    pub fn volts(&self, node: &str) -> Option<f64> {
        let key = node.to_ascii_lowercase();
        if key == "0" || key == "gnd" {
            return self.nodes.get(&key).copied().or(Some(0.0));
        }
        self.nodes.get(&key).copied()
    }

    /// `V(a) − V(b)`.
    pub fn voltage_between(&self, a: &str, b: &str) -> Option<f64> {
        Some(self.volts(a)? - self.volts(b)?)
    }

    /// The unique part joining `a` and `b`, and the current from `a` toward `b`.
    pub fn unique_current(&self, a: &str, b: &str) -> Result<(String, f64), AmpError> {
        let a = a.to_ascii_lowercase();
        let b = b.to_ascii_lowercase();
        if a == b {
            return Err(AmpError::SameNet);
        }
        let mut found: Vec<(&str, f64)> = Vec::new();
        for br in &self.branches {
            if br.a == a && br.b == b {
                found.push((&br.name, br.amps));
            } else if br.a == b && br.b == a {
                found.push((&br.name, -br.amps));
            }
        }
        match found.as_slice() {
            [(name, amps)] => Ok(((*name).to_owned(), *amps)),
            _ => Err(AmpError::NoUniqueBranch),
        }
    }
}

/// Live value of a placed meter.
#[cfg(test)]
#[derive(Debug, Clone, PartialEq)]
pub enum MeterReading {
    Volts(f64),
    Amps { amps: f64, part: String },
}

#[cfg(test)]
impl MeterReading {
    pub fn display(&self) -> String {
        match self {
            Self::Volts(v) => format!("{v:.3} V"),
            Self::Amps { amps, .. } => {
                let ma = amps * 1e3;
                if ma.abs() >= 100.0 {
                    format!("{ma:.0} mA")
                } else {
                    format!("{ma:.2} mA")
                }
            }
        }
    }
}

/// Fill voltages for the named nodes (missing nodes are skipped).
#[cfg(test)]
pub fn take_nodes(analog: &of_analog::AnalogEngine, names: &[&str], out: &mut CircuitReadout) {
    out.set_node("0", 0.0);
    out.set_node("gnd", 0.0);
    for name in names {
        let v = analog.value(&format!("v({name})")).or_else(|| analog.value(name));
        if let Some(v) = v {
            out.set_node(name, v);
        }
    }
}

/// Series LED current equals the resistor current (the only path).
#[cfg(test)]
pub fn add_led_series(out: &mut CircuitReadout, resistor: &str, diode: &str, gpio: &str, anode: &str, ohms: f64) {
    let Some(ir) = out.voltage_between(gpio, anode).map(|v| v / ohms) else {
        return;
    };
    out.push_branch(resistor, gpio, anode, ir);
    out.push_branch(diode, anode, "0", ir);
}

/// Record the merged PSU branch using the sourced current from `SupplyStatus::amps`
/// (positive when the instrument sources into the rail). The CurrentSource used in
/// CC mode has no `i()` probe, so callers pass `supply_status.amps` rather than
/// summing `i(psu)` and `i(psu_cc)`.
#[cfg(test)]
pub fn add_psu(out: &mut CircuitReadout, sourced_amps: f64) {
    // Current from vdd toward 0 through the instrument is the opposite of the sourced current.
    out.push_branch("PSU", "vdd", "0", -sourced_amps);
}

#[cfg(test)]
mod tests {
    use super::*;

    const R_SERIES: f64 = 330.0;

    #[test]
    fn voltmeter_is_the_difference_of_node_voltages() {
        let mut r = CircuitReadout::new();
        r.set_node("vdd", 3.3);
        r.set_node("gpio2", 1.65);
        assert!((r.voltage_between("gpio2", "0").unwrap() - 1.65).abs() < 1e-12);
        assert!((r.voltage_between("gpio2", "vdd").unwrap() + 1.65).abs() < 1e-12);
        assert!((r.voltage_between("vdd", "vdd").unwrap()).abs() < 1e-12);
    }

    #[test]
    fn ammeter_needs_a_unique_branch() {
        let mut r = CircuitReadout::new();
        r.set_node("gpio8", 3.3);
        r.set_node("led_a", 2.0);
        r.push_branch("R1", "gpio8", "led_a", (3.3 - 2.0) / R_SERIES);
        r.push_branch("D1", "led_a", "0", (3.3 - 2.0) / R_SERIES);
        r.push_branch("PSU", "vdd", "0", -0.01);
        // A second hidden path on the same nodes would be rejected.
        assert!(r.unique_current("gpio8", "led_a").is_ok());
        assert_eq!(r.unique_current("gpio8", "gpio8"), Err(AmpError::SameNet));
        assert_eq!(r.unique_current("gpio8", "0"), Err(AmpError::NoUniqueBranch));
        r.push_branch("extra", "gpio8", "led_a", 0.0);
        assert_eq!(r.unique_current("gpio8", "led_a"), Err(AmpError::NoUniqueBranch));
    }
}
