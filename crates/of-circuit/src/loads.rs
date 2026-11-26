//! Real-world instrument loading (feature `engine`, engine E5f, RFC 0004).
//!
//! An `[[instrument]]` with a real-world profile adds its input to the
//! circuit: a voltmeter or scope probe a shunt ([`LoadElement::Shunt`]), an
//! ammeter its burden in series with the measured pin
//! ([`LoadElement::Series`]). The circuit is re-elaborated with those
//! elements as ordinary resistors and capacitors named under the
//! instrument's ref, so adding a series ammeter rebuilds the session (it
//! changes the topology); ideal profiles add nothing.

use of_instruments::Signal;
use of_instruments::profile::{AmpInput, LoadElement, Profile, lookup};
use toml::Spanned;

use crate::library::PartsIndex;
use crate::validate::is_inline_signal;
use crate::{CircuitFile, ComponentEntry, NetEntry, NetKind, ParamValue};

/// Part used for load resistances.
pub const LOAD_RESISTOR: &str = "generic.resistor";
/// Part used for load capacitances.
pub const LOAD_CAPACITOR: &str = "generic.capacitor-mlcc";

/// One element an instrument adds.
#[derive(Debug, Clone, PartialEq)]
pub struct InstrumentLoad {
    /// The instrument's ref.
    pub owner: String,
    /// What it adds.
    pub element: LoadElement,
}

/// The multimeter current input of an instrument (`params.input`, default mA).
pub fn amp_input(params: &std::collections::BTreeMap<String, ParamValue>) -> AmpInput {
    match params.get("input") {
        Some(ParamValue::Text(t)) => AmpInput::parse(t).unwrap_or(AmpInput::Milli),
        _ => AmpInput::Milli,
    }
}

/// The loads of every real-world instrument in `file`, in file order.
///
/// A multimeter in `dc_v` puts its input resistance across the measured
/// nets, in `dc_a` its burden in series with the measured pin; a scope puts
/// its input from each probed net to ground (differential and current
/// probes add nothing). Unknown signals are skipped (they are reported by
/// `library::check_signals`).
pub fn instrument_loads(file: &CircuitFile, lib: &PartsIndex) -> Vec<InstrumentLoad> {
    let ground = file.circuit.ground.get_ref().as_str();
    let mut out = Vec::new();
    for i in &file.instruments {
        let Some(profile) = lookup(i.profile()) else {
            continue;
        };
        let owner = i.reference().to_owned();
        for p in &i.probes {
            let Some(sig) = signal(file, p.get_ref()) else {
                continue;
            };
            let element = match (profile, &sig) {
                (Profile::Dmm(d), Signal::NetVolts(n)) if i.mode() == Some("dc_v") => d.volts_load(n, ground),
                (Profile::Dmm(d), Signal::DiffVolts(a, b)) if i.mode() == Some("dc_v") => d.volts_load(a, b),
                (Profile::Dmm(d), Signal::PinCurrent { refdes, pin }) if i.mode() == Some("dc_a") => {
                    let Some(pin) = pin.clone().or_else(|| first_pin(file, lib, refdes)) else {
                        continue;
                    };
                    d.amps_load(amp_input(&i.params), refdes, &pin)
                }
                (Profile::Scope(s), Signal::NetVolts(n)) => s.load(n, ground),
                _ => continue,
            };
            out.push(InstrumentLoad {
                owner: owner.clone(),
                element,
            });
        }
    }
    out
}

/// The signal a probe entry names (inline or through `[[probe]]`).
fn signal(file: &CircuitFile, text: &str) -> Option<Signal> {
    let text = if is_inline_signal(text) {
        text
    } else {
        file.probes.iter().find(|p| p.name() == text)?.signal()
    };
    text.parse().ok()
}

/// The first pin of `refdes`'s part.
fn first_pin(file: &CircuitFile, lib: &PartsIndex, refdes: &str) -> Option<String> {
    let c = file.components.iter().find(|c| c.reference() == refdes)?;
    Some(lib.get(c.part_id())?.pins.first()?.name.clone())
}

/// `file` with `loads` added as components and nets.
///
/// Shunt `k` of instrument `X` is `X_R<k>` (and `X_C<k>` when it has a
/// capacitance) between the two nets; series element `k` moves the pin to a
/// new net `X_n<k>` and joins it to the old net through `X_R<k>`.
pub fn with_loads(file: &CircuitFile, loads: &[InstrumentLoad]) -> Result<CircuitFile, String> {
    let mut out = file.clone();
    let ground = file.circuit.ground.get_ref().clone();
    let net_name = |n: &str| if n == "0" { ground.clone() } else { n.to_owned() };
    let mut k = std::collections::BTreeMap::<&str, usize>::new();
    for l in loads {
        let n = k.entry(l.owner.as_str()).or_insert(0);
        *n += 1;
        let r = format!("{}_R{n}", l.owner);
        match &l.element {
            LoadElement::Shunt {
                net,
                reference,
                ohms,
                farads,
            } => {
                let (a, b) = (net_name(net), net_name(reference));
                add_part(&mut out, &r, LOAD_RESISTOR, "resistance", *ohms);
                attach(&mut out, &a, format!("{r}.1"))?;
                attach(&mut out, &b, format!("{r}.2"))?;
                if *farads > 0.0 {
                    let c = format!("{}_C{n}", l.owner);
                    add_part(&mut out, &c, LOAD_CAPACITOR, "capacitance", *farads);
                    attach(&mut out, &a, format!("{c}.1"))?;
                    attach(&mut out, &b, format!("{c}.2"))?;
                }
            }
            LoadElement::Series { refdes, pin, ohms } => {
                let terminal = format!("{refdes}.{pin}");
                let old = out
                    .nets
                    .iter_mut()
                    .find(|net| net.pins.iter().any(|p| p.get_ref() == &terminal))
                    .ok_or_else(|| format!("{}: {terminal} is not on a net", l.owner))?;
                old.pins.retain(|p| p.get_ref() != &terminal);
                old.pins.push(Spanned::new(0..0, format!("{r}.1")));
                add_part(&mut out, &r, LOAD_RESISTOR, "resistance", *ohms);
                out.nets.push(NetEntry {
                    name: Spanned::new(0..0, format!("{}_n{n}", l.owner)),
                    label: None,
                    kind: NetKind::default(),
                    pins: vec![Spanned::new(0..0, terminal), Spanned::new(0..0, format!("{r}.2"))],
                    waypoints: Vec::new(),
                });
            }
        }
    }
    Ok(out)
}

fn add_part(file: &mut CircuitFile, reference: &str, part: &str, param: &str, value: f64) {
    file.components.push(ComponentEntry {
        reference: Spanned::new(0..0, reference.to_owned()),
        part: Spanned::new(0..0, part.to_owned()),
        at: None,
        params: [(param.to_owned(), ParamValue::Number(value))].into(),
        seed: None,
        firmware: None,
        schematic: None,
    });
}

fn attach(file: &mut CircuitFile, net: &str, terminal: String) -> Result<(), String> {
    let n = file
        .nets
        .iter_mut()
        .find(|n| n.name() == net)
        .ok_or_else(|| format!("no net '{net}' to load"))?;
    n.pins.push(Spanned::new(0..0, terminal));
    Ok(())
}
