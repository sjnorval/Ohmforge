//! Circuit file to a runnable engine (feature `engine`, RFC 0003 "Building").
//!
//! [`build_engine`] checks the file against the parts library and its
//! firmware hashes, resolves firmware through a [`Resolver`], builds every
//! component through the model [`Registry`] in file order and elaborates the
//! analog circuit. [`build`] then starts an [`of_engine::Session`] with the
//! file's `[sim]` quantum and step. Instruments with a real-world profile add
//! their inputs first ([`crate::loads`]), so adding one rebuilds the session.
//!
//! The registry decides the corner and temperature of library parts (see
//! [`corner`]); the circuit only names parts, params, nets and firmware.

use std::collections::BTreeMap;

use of_component_api::ParamValue as ApiParam;
use of_core::SimTime;
use of_engine::{BuiltEngine, ComponentSpec, EngineBuilder, Firmware, Registry, Session, SessionConfig};
use toml::Spanned;

use crate::library::{PartsIndex, Resolver, check_firmware, check_parts, pin_named};
use crate::validate::{RefTarget, ref_table, split_pin};
use crate::{BootKind, CircuitError, CircuitFile, ComponentEntry, FirmwareRef, ParamValue, SimCorner};

/// A runnable circuit: the session and the file it came from.
pub struct Built {
    /// The running circuit.
    pub session: Session,
    /// The circuit file.
    pub file: CircuitFile,
}

/// The parts-library corner for a `[sim] corner`.
pub fn corner(c: SimCorner) -> of_parts::Corner {
    match c {
        SimCorner::Typ => of_parts::Corner::Typical,
        SimCorner::Min => of_parts::Corner::Min,
        SimCorner::Max => of_parts::Corner::Max,
    }
}

/// The session settings of a `[sim]` table.
pub fn session_config(file: &CircuitFile) -> SessionConfig {
    SessionConfig {
        quantum: SimTime::from_us(file.sim.quantum_us),
        max_step: file.sim.max_step,
    }
}

/// Build and start `file` (parsed from `src`, which error positions refer to).
pub fn build(
    src: &str,
    file: &CircuitFile,
    parts: &PartsIndex,
    reg: &Registry,
    res: &dyn Resolver,
) -> Result<Built, CircuitError> {
    let engine = build_engine(src, file, parts, reg, res)?;
    let session = engine
        .into_session(session_config(file))
        .map_err(|e| CircuitError::general(format!("cannot start the circuit: {e}")))?;
    Ok(Built {
        session,
        file: file.clone(),
    })
}

/// Check, resolve and elaborate `file` without starting it.
///
/// Errors carry the line and column of the offending entry: an unknown part
/// or pin, a firmware file that cannot be read or whose SHA-256 does not
/// match (the message shows the actual hash), a model that is not
/// registered, or a component that rejects its params.
pub fn build_engine(
    src: &str,
    file: &CircuitFile,
    parts: &PartsIndex,
    reg: &Registry,
    res: &dyn Resolver,
) -> Result<BuiltEngine, CircuitError> {
    // Real-world instruments re-elaborate the circuit with their inputs (E5f).
    let loaded =
        crate::loads::with_loads(file, &crate::loads::instrument_loads(file, parts)).map_err(CircuitError::general)?;
    let file = &loaded;
    if let Some(e) = check_parts(src, file, parts).into_iter().next() {
        return Err(e);
    }
    if let Some(e) = check_firmware(src, file, res).into_iter().next() {
        return Err(e);
    }
    let ground = &file.circuit.ground;
    if !matches!(ground.get_ref().to_ascii_lowercase().as_str(), "gnd" | "0") {
        return Err(CircuitError::at(
            src,
            ground.span(),
            format!(
                "ground net '{}': the engine needs the ground net to be named 'gnd' for now",
                ground.get_ref()
            ),
        ));
    }

    // Electrical pins of each component, in net order then pin order.
    let refs = ref_table(file);
    let mut pins: Vec<Vec<(String, String)>> = vec![Vec::new(); file.components.len()];
    for net in &file.nets {
        for p in &net.pins {
            let Some((r, pin)) = split_pin(p.get_ref()) else {
                continue;
            };
            let Some(&(RefTarget::Component(i), _)) = refs.get(&r.to_lowercase()) else {
                continue;
            };
            let part = parts.get(file.components[i].part_id()).expect("checked by check_parts");
            let name = pin_named(part, pin).unwrap_or(pin);
            pins[i].push((name.to_owned(), net.name().to_owned()));
        }
    }

    let mut eng = EngineBuilder::new(ground.get_ref());
    for net in &file.nets {
        eng.net(net.name());
    }
    for (c, pins) in file.components.iter().zip(pins) {
        add_component(src, file, c, &pins, parts, reg, res, &mut eng)?;
    }
    eng.elaborate()
        .map_err(|e| CircuitError::general(format!("the circuit does not elaborate: {e}")))
}

#[allow(clippy::too_many_arguments)]
fn add_component(
    src: &str,
    file: &CircuitFile,
    c: &ComponentEntry,
    pins: &[(String, String)],
    parts: &PartsIndex,
    reg: &Registry,
    res: &dyn Resolver,
    eng: &mut EngineBuilder,
) -> Result<(), CircuitError> {
    let r = c.reference();
    let at_ref = |msg: String| CircuitError::at(src, c.reference.span(), msg);
    let part = parts.get(c.part_id()).expect("checked by check_parts");
    let model = part.part.model.as_str();
    if !reg.contains(model) {
        return Err(CircuitError::at(
            src,
            c.part.span(),
            format!(
                "{r}: part '{}' uses model '{model}', which is not registered",
                c.part_id()
            ),
        ));
    }
    let firmware = match &c.firmware {
        Some(fw) => Some(resolve_firmware(src, c, fw, res)?),
        None => None,
    };
    let params: BTreeMap<String, ApiParam> = c.params.iter().map(|(k, v)| (k.clone(), api_param(v))).collect();
    let pin_names: Vec<String> = pins.iter().map(|(p, _)| p.clone()).collect();
    let seed = c.seed.unwrap_or(file.sim.seed);
    let spec = ComponentSpec {
        refdes: r,
        part_id: c.part_id(),
        params: &params,
        firmware,
        pins: &pin_names,
        seed,
    };
    let numeric = spec.numeric_params();
    let component = reg.build(model, spec).map_err(|e| at_ref(e.to_string()))?;
    let pin_nets: Vec<(&str, &str)> = pins.iter().map(|(p, n)| (p.as_str(), n.as_str())).collect();
    let num: Vec<(&str, f64)> = numeric.iter().map(|(k, v)| (k.as_str(), *v)).collect();
    eng.add(component, &pin_nets, &num, seed)
        .map_err(|e| at_ref(e.to_string()))?;
    Ok(())
}

fn api_param(v: &ParamValue) -> ApiParam {
    match v {
        ParamValue::Number(x) => ApiParam::Number(*x),
        ParamValue::Bool(b) => ApiParam::Bool(*b),
        ParamValue::Text(t) => ApiParam::Text(t.clone()),
        ParamValue::List(l) => ApiParam::List(l.clone()),
    }
}

fn resolve_firmware(
    src: &str,
    c: &ComponentEntry,
    fw: &FirmwareRef,
    res: &dyn Resolver,
) -> Result<Firmware, CircuitError> {
    let r = c.reference();
    let read = |p: &Spanned<String>| {
        res.read(p.get_ref())
            .map_err(|e| CircuitError::at(src, p.span(), format!("{r}: cannot read firmware: {e}")))
    };
    let at_ref = |msg: String| CircuitError::at(src, c.reference.span(), format!("{r}: {msg}"));
    match fw.boot {
        BootKind::Direct => Ok(Firmware::Elf(read(&fw.app)?)),
        BootKind::Rom => {
            if let Some(ready) = &fw.ready_text
                && let Some(booted) = res
                    .booted(c.part_id(), fw.app_sha256.get_ref(), ready)
                    .map_err(at_ref)?
            {
                return Ok(booted);
            }
            let app = read(&fw.app)?;
            let Some(p) = &fw.partitions else {
                return Err(CircuitError::at(
                    src,
                    fw.app.span(),
                    format!("{r}: boot = \"rom\" needs a partition table (`partitions`)"),
                ));
            };
            let partitions = read(p)?;
            let rom = res.rom(c.part_id()).map_err(at_ref)?;
            Ok(Firmware::Rom {
                app,
                partitions,
                rom: rom.rom,
                bootloader: rom.bootloader,
                ready_text: fw.ready_text.clone(),
            })
        }
    }
}
