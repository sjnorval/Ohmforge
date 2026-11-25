//! `ohmforge run <file.circuit.toml>`: build a circuit file through the
//! model registry and run it headless (engine E4b, #48).
//!
//! Every non-ground net voltage (file order) and then every `[[probe]]`
//! signal not already listed is sampled every `--step` until `--until`.
//! The CSV (and VCD) writers are the SPICE path's. `i(REF)` of a resistor
//! is computed from its pin voltages when the solver has no branch current.

use std::fs::File;
use std::io::{self, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::sync::Arc;

use of_analog::Waveforms;
use of_circuit::build::{Built, build, corner};
use of_circuit::fs::{FsResolver, RomSource, load_parts, read_circuit};
use of_components::analog::{Env, Library};
use of_components::register_builtin;
use of_core::{SimTime, Stop};
use of_engine::Registry;
use of_soc_esp32c3::Esp32c3Engine;

/// The pinned ESP32-C3 ROM release (`scripts/fetch-esp-rom-elfs.sh`).
const C3_ROM: RomSource = RomSource {
    release: of_soc_esp32c3::rom::ROM_ELFS_RELEASE,
    rom_file: of_soc_esp32c3::rom::C3_ROM_FILE,
    rom_sha256: of_soc_esp32c3::rom::C3_ROM_SHA256,
    bootloader_file: of_soc_esp32c3::bootloader::C3_BOOTLOADER_FILE,
    bootloader_sha256: of_soc_esp32c3::bootloader::C3_BOOTLOADER_SHA256,
};

/// Options of `run` for a circuit file.
#[derive(Debug)]
pub struct Args {
    pub file: PathBuf,
    pub until: f64,
    pub step: f64,
    pub out: Option<PathBuf>,
    pub vcd: Option<PathBuf>,
    pub uart: bool,
    pub parts: PathBuf,
}

/// True for `*.circuit.toml` paths.
pub fn is_circuit_file(path: &Path) -> bool {
    path.file_name()
        .is_some_and(|n| n.to_string_lossy().ends_with(of_circuit::fs::SUFFIX))
}

/// A time such as `10ms`, `250us`, `1.5e-3` (seconds) or `2s`.
pub fn parse_time(s: &str) -> Result<f64, String> {
    let t = s.trim();
    let (num, scale) = [("ns", 1e-9), ("us", 1e-6), ("\u{b5}s", 1e-6), ("ms", 1e-3), ("s", 1.0)]
        .iter()
        .find_map(|(suffix, k)| t.strip_suffix(suffix).map(|n| (n, *k)))
        .unwrap_or((t, 1.0));
    let v: f64 = num
        .trim()
        .parse()
        .map_err(|_| format!("'{s}' is not a time (for example 10ms, 250us or 1e-3)"))?;
    if !v.is_finite() || v <= 0.0 {
        return Err(format!("'{s}' must be a positive time"));
    }
    Ok(v * scale)
}

/// Parse the options after `run <file.circuit.toml>`.
pub fn parse_args(file: PathBuf, rest: &[String]) -> Result<Args, String> {
    let mut a = Args {
        file,
        until: 10e-3,
        step: 10e-6,
        out: None,
        vcd: None,
        uart: false,
        parts: PathBuf::from("parts"),
    };
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let mut value = |what: &str| it.next().cloned().ok_or(format!("{arg} needs {what}"));
        match arg.as_str() {
            "--until" => a.until = parse_time(&value("a time")?)?,
            "--step" => a.step = parse_time(&value("a time")?)?,
            "--out" | "-o" => a.out = Some(value("a file name")?.into()),
            "--vcd" => a.vcd = Some(value("a file name")?.into()),
            "--parts" => a.parts = value("a directory")?.into(),
            "--uart" => a.uart = true,
            s if s.starts_with('-') => return Err(format!("unknown option '{s}'")),
            s => return Err(format!("unexpected argument '{s}'")),
        }
    }
    if a.step > a.until {
        return Err("--step is longer than --until".into());
    }
    Ok(a)
}

fn registry(parts: &Path, file: &of_circuit::CircuitFile) -> Result<Registry, String> {
    let lib = Library::load_dir(parts).map_err(|e| format!("parts library: {e} (use --parts DIR)"))?;
    let env = Env {
        temp_c: file.sim.temperature_c.unwrap_or(Env::default().temp_c),
        ..Env::default()
    };
    let mut reg = Registry::new();
    register_builtin(&mut reg, Arc::new(lib), corner(file.sim.corner), env);
    of_soc_esp32c3::register(&mut reg);
    Ok(reg)
}

/// Sampled signals: non-ground net voltages, then probes not yet listed.
fn signals(file: &of_circuit::CircuitFile) -> Vec<String> {
    let ground = file.circuit.ground.get_ref().to_lowercase();
    let mut out: Vec<String> = file
        .nets
        .iter()
        .filter(|n| n.name().to_lowercase() != ground)
        .map(|n| format!("v({})", n.name()))
        .collect();
    for p in &file.probes {
        if !out.iter().any(|s| s.eq_ignore_ascii_case(&p.signal)) {
            out.push(p.signal.clone());
        }
    }
    out
}

/// `i(REF)` of a two-pin resistor, from pin 1 to pin 2.
fn resistor_current(b: &mut Built, signal: &str) -> Option<f64> {
    let inner = signal.strip_prefix("i(").or_else(|| signal.strip_prefix("I("))?;
    let reference = inner.strip_suffix(')')?;
    let c = b
        .file
        .components
        .iter()
        .find(|c| c.reference().eq_ignore_ascii_case(reference))?;
    let r = c.params.get("resistance")?.as_number()?;
    let ground = b.file.circuit.ground.get_ref().to_lowercase();
    let net = |pin: &str| {
        let want = format!("{reference}.{pin}").to_lowercase();
        b.file
            .nets
            .iter()
            .find(|n| n.pins().any(|p| p.to_lowercase() == want))
            .map(|n| n.name().to_lowercase())
    };
    let (n1, n2) = (net("1")?, net("2")?);
    let mut v = |n: &str| {
        if n == ground {
            Some(0.0)
        } else {
            b.session.value(&format!("v({n})"))
        }
    };
    Some((v(&n1)? - v(&n2)?) / r)
}

fn run_to(b: &mut Built, t: f64) -> Result<(), String> {
    let target = SimTime::from_secs_f64(t);
    while b.session.now() < target {
        match b.session.run_until(target).map_err(|e| e.to_string())? {
            Stop::Reached(_) | Stop::Breakpoint(_) | Stop::Host(_) => {}
            Stop::Requested(id) => {
                return Err(format!(
                    "engine {id:?} stopped the run at {:.6} ms",
                    b.session.now().as_secs_f64() * 1e3
                ));
            }
        }
    }
    b.session.commit().map_err(|e| e.to_string())?;
    Ok(())
}

fn write(path: &Path, f: impl FnOnce(&mut dyn Write) -> io::Result<()>) -> Result<(), String> {
    let mut w = BufWriter::new(File::create(path).map_err(|e| format!("{}: {e}", path.display()))?);
    f(&mut w)
        .and_then(|()| w.flush())
        .map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("wrote {}", path.display());
    Ok(())
}

/// Run one circuit file.
pub fn run(a: &Args) -> Result<(), String> {
    let (src, file) = read_circuit(&a.file).map_err(|e| e.to_string())?;
    let parts = load_parts(&a.parts).map_err(|e| format!("parts library: {e} (use --parts DIR)"))?;
    let reg = registry(&a.parts, &file)?;
    let res = FsResolver::for_file(&a.file).with_rom("espressif.esp32-c3", C3_ROM);
    let mut b = build(&src, &file, &parts, &reg, &res).map_err(|e| e.with_path(&a.file).to_string())?;

    let names = signals(&b.file);
    let mut x = Vec::new();
    let mut columns = vec![Vec::new(); names.len()];
    let mut missing = vec![false; names.len()];
    let steps = (a.until / a.step).round().max(1.0) as u64;
    for k in 0..=steps {
        let t = (k as f64 * a.step).min(a.until);
        if k > 0 {
            run_to(&mut b, t)?;
        }
        x.push(b.session.now().as_secs_f64());
        for (i, n) in names.iter().enumerate() {
            let v = b.session.value(n).or_else(|| resistor_current(&mut b, n));
            if v.is_none() && !missing[i] {
                eprintln!("warning: no signal '{n}' in {}", a.file.display());
                missing[i] = true;
            }
            columns[i].push(v.unwrap_or(f64::NAN));
        }
    }
    eprintln!(
        "{}: {} samples to {:.6} s",
        b.file.circuit.name,
        x.len(),
        b.session.now().as_secs_f64()
    );
    let waves = Waveforms::from_columns("time", x, names, columns);
    match &a.out {
        Some(p) => write(p, |w| waves.write_csv(w))?,
        None if !a.uart => waves.write_csv(&mut io::stdout().lock()).map_err(|e| e.to_string())?,
        None => {}
    }
    if let Some(p) = &a.vcd {
        write(p, |w| waves.write_vcd(w))?;
    }
    if a.uart {
        let mcus: Vec<String> = b
            .file
            .components
            .iter()
            .filter(|c| parts.get(c.part_id()).is_some_and(|p| p.part.model == "rust:esp32c3"))
            .map(|c| c.reference().to_owned())
            .collect();
        for r in mcus {
            if let Some(cpu) = b.session.engine_as::<Esp32c3Engine>(&r) {
                let text = cpu.soc.uart_text();
                let mut out = io::stdout().lock();
                write!(out, "{text}").map_err(|e| e.to_string())?;
                if !text.is_empty() && !text.ends_with('\n') {
                    writeln!(out).map_err(|e| e.to_string())?;
                }
            }
        }
    }
    Ok(())
}
