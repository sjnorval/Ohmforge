//! The desktop demos on [`of_engine::Session`], built from their circuit
//! files (engine E4c; the desktop's only path since E4d).
//!
//! [`SessionSim`] runs a built `Session` with the desktop's sampling loops
//! (scope grid, LEDC events, the button's grid; the loops of the hand-wired
//! demos it replaced) and offers the methods the live worker calls. The trace
//! parity tests (`parity`) run the same type, so the desktop's path is the
//! code the goldens check. [`start`] builds a demo from the embedded
//! `circuits/demos` file with the embedded parts and firmware; the ROM-booted
//! chips come from the pre-boot caches in `adc_pwm` and `button`.
//!
//! The shipped files keep the ESP32-C3's supply current (`quiescent_a`, from
//! datasheet v2.5 Table 5-8).

use std::sync::{Arc, OnceLock};

use of_circuit::build::{build, corner};
use of_circuit::demos::{self, DemoFile};
use of_circuit::library::PartsIndex;
use of_circuit::{CircuitFile, ParamValue};
use of_components::analog::{Env, Library};
use of_components::{BounceSettings, register_builtin};
use of_core::{SimTime, Stop};
use of_engine::{Firmware, Registry, Session};
use of_instruments::{
    Averaging, ChannelId, MeasurementService, MeterMode, Multimeter, PartPins, Probeable, PsuReadout, Reading,
    SamplePlan, Scope, ScopeTrace, Signal,
};
use of_soc_esp32c3::{Esp32c3Engine, UartByte};

use crate::adc_pwm::{self, PwmKind};
use crate::button::{self, ButtonKind};
use crate::kind::DemoKind;
use crate::supply::{SupplySettings, SupplyStatus};
use crate::trace;

/// Display-average window of the PWM demos (as `PwmSim`).
const PWM_AVG_WINDOW: f64 = 2e-3;
/// Display-average window of the button demos (as `ButtonSim`).
const BUTTON_AVG_WINDOW: f64 = 2e-3;
const APB_HZ: u64 = 80_000_000;

/// The signal of `kind`'s `[[probe]] name = "LED current"` (E5b: the scope
/// and averages read it through the measurement service).
fn led_probe(kind: DemoKind) -> Result<Signal, String> {
    let file = parsed(kind)?;
    let probe = file
        .probes
        .iter()
        .find(|p| p.name() == "LED current")
        .ok_or_else(|| format!("{}: no `LED current` probe", file_stem(kind)))?;
    probe.signal().parse().map_err(|e| format!("{}: {e}", file_stem(kind)))
}

/// The circuit file stem of `kind` (`circuits/demos/<stem>.circuit.toml`).
pub(crate) fn file_stem(kind: DemoKind) -> &'static str {
    match kind {
        DemoKind::Blink => "blink",
        DemoKind::Dimmer => "dimmer",
        DemoKind::Fade => "fade",
        DemoKind::ButtonPoll => "button",
        DemoKind::ButtonIrq => "button-irq",
        DemoKind::Ne555Astable => "ne555-astable",
        DemoKind::Ne555Monostable => "ne555-monostable",
        DemoKind::Ne555Pwm => "ne555-pwm-dimmer",
        DemoKind::Clock555 => "esp32c3-555-clock",
    }
}

/// The embedded file of `kind`.
pub(crate) fn demo_file(kind: DemoKind) -> &'static DemoFile {
    demos::demo(file_stem(kind)).expect("every desktop demo has an embedded circuit file")
}

const KINDS: [DemoKind; 9] = [
    DemoKind::Blink,
    DemoKind::Dimmer,
    DemoKind::Fade,
    DemoKind::ButtonPoll,
    DemoKind::ButtonIrq,
    DemoKind::Ne555Astable,
    DemoKind::Ne555Monostable,
    DemoKind::Ne555Pwm,
    DemoKind::Clock555,
];

/// The parsed embedded file of `kind` (parsed once).
pub(crate) fn parsed(kind: DemoKind) -> Result<&'static CircuitFile, String> {
    static FILES: [OnceLock<Result<CircuitFile, String>>; 9] = [const { OnceLock::new() }; 9];
    let i = KINDS.iter().position(|k| *k == kind).expect("known kind");
    FILES[i]
        .get_or_init(|| {
            let d = demo_file(kind);
            of_circuit::parse(d.text).map_err(|e| format!("{}: {e}", d.path))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The embedded demo parts, as the builder's index and the models' library.
fn parts() -> Result<&'static (PartsIndex, Arc<Library>), String> {
    static PARTS: OnceLock<Result<(PartsIndex, Arc<Library>), String>> = OnceLock::new();
    PARTS
        .get_or_init(|| {
            let files = demos::part_files()?;
            let mut lib = Library::new();
            for p in &files {
                let spice = demos::spice_model(&p.part.model).map(str::to_owned);
                lib.add(p.clone(), spice).map_err(|e| e.to_string())?;
            }
            Ok((PartsIndex::new(files), Arc::new(lib)))
        })
        .as_ref()
        .map_err(Clone::clone)
}

/// The built-in models over the embedded parts, at the file's corner and
/// temperature.
fn registry(file: &CircuitFile, lib: Arc<Library>) -> Registry {
    let env = Env {
        temp_c: file.sim.temperature_c.unwrap_or(Env::default().temp_c),
        ..Env::default()
    };
    let mut reg = Registry::new();
    register_builtin(&mut reg, lib, corner(file.sim.corner), env);
    of_soc_esp32c3::register(&mut reg);
    reg
}

/// The pre-booted chip of a ROM demo (the `adc_pwm` / `button` caches), or
/// `None` for the direct-boot blink. The error is the missing-ROM message.
pub(crate) fn booted(kind: DemoKind) -> Result<Option<Firmware>, String> {
    let soc = match kind {
        DemoKind::Blink | DemoKind::Clock555 => return Ok(None),
        DemoKind::Dimmer => adc_pwm::booted(PwmKind::Dimmer)?,
        DemoKind::Fade => adc_pwm::booted(PwmKind::Fade)?,
        DemoKind::ButtonPoll => button::booted(ButtonKind::Poll)?,
        DemoKind::ButtonIrq => button::booted(ButtonKind::Irq)?,
        DemoKind::Ne555Astable | DemoKind::Ne555Monostable | DemoKind::Ne555Pwm => return Ok(None),
    };
    Ok(Some(Firmware::Booted(Box::new(soc))))
}

/// Overrides applied to the file before it is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StartOptions {
    /// Bench supply settings (`PSU` params); `None` keeps the file's own
    /// ([`file_supply`]), so every demo starts on the supply its circuit sets.
    pub supply: Option<SupplySettings>,
    /// Pot wiper position (`RV1 position`, dimmer only).
    pub position: f64,
    /// Keep the file's ESP32-C3 supply current (`false` forces
    /// `quiescent_a = 0`, as the parity runs do on both paths).
    pub idle_draw: bool,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            supply: None,
            position: 0.5,
            idle_draw: true,
        }
    }
}

/// The bench supply as `kind`'s file sets it (`PSU` params over the
/// defaults): the 555 demos run at 5 V (NE555 4.5 to 16 V), not the
/// ESP32-C3's 3.3 V.
pub(crate) fn file_supply(kind: DemoKind) -> SupplySettings {
    let mut s = SupplySettings::default();
    let Ok(file) = parsed(kind) else {
        return s;
    };
    let Some(psu) = file.components.iter().find(|c| c.reference() == "PSU") else {
        return s;
    };
    let num = |k: &str| match psu.params.get(k) {
        Some(ParamValue::Number(x)) => Some(*x),
        Some(ParamValue::Bool(b)) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    };
    if let Some(v) = num("volts") {
        s.volts = v;
    }
    if let Some(r) = num("series_r") {
        s.series_r = r;
    }
    if let Some(i) = num("current_limit") {
        s.current_limit = i;
    }
    if let Some(on) = num("on") {
        s.on = on != 0.0;
    }
    s.sanitized()
}

/// The parts on the + rail of `kind`'s bench supply that state a supply
/// range (`library::rail_parts`): the parts a supply warning may name.
pub(crate) fn rail_parts(kind: DemoKind) -> Vec<of_circuit::library::RailPart> {
    let (Ok(file), Ok((index, _))) = (parsed(kind), parts()) else {
        return Vec::new();
    };
    let Some(psu) = file
        .components
        .iter()
        .find(|c| index.get(c.part_id()).is_some_and(|p| p.part.model == "rust:bench-psu"))
    else {
        return Vec::new();
    };
    let plus = format!("{}.+", psu.reference());
    let Some(net) = file
        .nets
        .iter()
        .find(|n| n.pins.iter().any(|p| p.get_ref().eq_ignore_ascii_case(&plus)))
    else {
        return Vec::new();
    };
    of_circuit::library::rail_parts(file, net.name(), psu.reference(), index)
}

fn set_param(file: &mut CircuitFile, reference: &str, name: &str, value: ParamValue) {
    if let Some(c) = file.components.iter_mut().find(|c| c.reference() == reference) {
        c.params.insert(name.into(), value);
    }
}

/// Build `kind` from its embedded circuit file.
pub(crate) fn build_session(kind: DemoKind, opts: StartOptions) -> Result<Session, String> {
    let d = demo_file(kind);
    let mut file = parsed(kind)?.clone();
    let s = opts.supply.unwrap_or_else(|| file_supply(kind)).sanitized();
    set_param(&mut file, "PSU", "volts", ParamValue::Number(s.volts));
    set_param(&mut file, "PSU", "series_r", ParamValue::Number(s.series_r));
    set_param(&mut file, "PSU", "current_limit", ParamValue::Number(s.current_limit));
    set_param(&mut file, "PSU", "on", ParamValue::Bool(s.on));
    if kind.has_pot() {
        set_param(
            &mut file,
            "RV1",
            "position",
            ParamValue::Number(opts.position.clamp(0.0, 1.0)),
        );
    }
    if !opts.idle_draw && kind.has_mcu() {
        set_param(&mut file, "U1", "quiescent_a", ParamValue::Number(0.0));
    }
    let (index, lib) = parts()?;
    let res = demos::resolver().with_booted(move |_, _, _| booted(kind));
    let built = build(d.text, &file, index, &registry(&file, Arc::clone(lib)), &res).map_err(|e| {
        // A missing ROM shows the desktop's own message.
        match booted(kind) {
            Err(rom) => rom,
            Ok(_) => format!("{}: {e}", d.path),
        }
    })?;
    Ok(built.session)
}

/// The sampling loop of a demo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Loop {
    /// `Blink::advance`: the scope grid.
    Blink,
    /// `PwmSim::advance`: the grid and LEDC events.
    Pwm,
    /// `ButtonSim::advance`: the grid, with bounce timers inside `Session`.
    Button,
    /// The 555 demos: the scope grid alone (no chip, as `Blink` without it).
    Analog,
}

impl Loop {
    pub(crate) fn of(kind: DemoKind) -> Self {
        match kind {
            DemoKind::Blink | DemoKind::Clock555 => Self::Blink,
            DemoKind::Dimmer | DemoKind::Fade => Self::Pwm,
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => Self::Button,
            DemoKind::Ne555Astable | DemoKind::Ne555Monostable | DemoKind::Ne555Pwm => Self::Analog,
        }
    }
}

/// A demo running on [`Session`] with the desktop's sampling loop.
pub(crate) struct SessionSim {
    kind: DemoKind,
    s: Session,
    style: Loop,
    /// The measurement service; samples the file's `LED current` probe (A).
    meas: MeasurementService,
    led: ChannelId,
    /// The scope on the LED channel (mA; PWM demos: whole-period moving average).
    scope: Scope,
    /// The placed meters' channels (E5c), with the LED probe again as the
    /// averaging reference ([`Self::meter_ref`]).
    meters: MeasurementService,
    meter_ref: ChannelId,
    /// Readings taken after each commit (the PSU face, E5d).
    settled: MeasurementService,
    psu: PsuReadout,
    /// Button loop: next grid sample (relative).
    next_sample: SimTime,
    closed: bool,
    bounce: BounceSettings,
    position: f64,
    supply: SupplySettings,
    supply_status: SupplyStatus,
    /// The file's UART consoles and logic analysers (E5g); empty on the
    /// shipped demos, so their runs are unchanged.
    digital: of_instruments::digital::DigitalBench,
    /// The file's scope and multimeter channels (the 555 demos).
    bench: MeasurementService,
    file_scopes: Vec<FileScope>,
    file_meters: Vec<FileMeter>,
}

/// The 555 demos' duty and frequency readout window (s): about 20 periods
/// of the PWM dimmer.
const ANALOG_PWM_WINDOW: f64 = 20e-3;

/// Each file scope's (reference, [(probe, samples)]).
pub(crate) type ScopeTraces = Vec<(String, Vec<(String, Vec<(f64, f64)>)>)>;

/// A file `scope` instrument on the desktop (the 555 demos): its probes'
/// channels on [`SessionSim::bench`], in the file's order.
pub(crate) struct FileScope {
    pub reference: String,
    pub channels: Vec<(String, ChannelId)>,
}

/// A file `multimeter` instrument: its mode and probe channel.
pub(crate) struct FileMeter {
    pub reference: String,
    pub mode: String,
    pub probe: String,
    pub channel: ChannelId,
}

/// The file's scope and multimeter instruments, watched on `bench`
/// (ESP32-C3 demos keep their own scope and meters, so none there).
fn file_instruments(
    kind: DemoKind,
    bench: &mut MeasurementService,
) -> Result<(Vec<FileScope>, Vec<FileMeter>), String> {
    let (mut scopes, mut meters) = (Vec::new(), Vec::new());
    if !kind.file_bench() {
        return Ok((scopes, meters));
    }
    let file = parsed(kind)?;
    for i in &file.instruments {
        let mut channels = Vec::new();
        for p in &i.probes {
            // A `[[probe]]` name, or an inline signal such as `v(vdd)`.
            let (name, text) = file
                .probe(p.get_ref())
                .map_or((p.get_ref().as_str(), p.get_ref().as_str()), |q| (q.name(), q.signal()));
            let signal: Signal = text.parse().map_err(|e| format!("{}: {e}", i.reference()))?;
            channels.push((name.to_owned(), bench.watch_default(signal)));
        }
        match i.kind() {
            "scope" => scopes.push(FileScope {
                reference: i.reference().to_owned(),
                channels,
            }),
            "multimeter" => {
                let mode = i
                    .mode
                    .as_ref()
                    .map_or_else(|| "dc_v".to_owned(), |m| m.get_ref().clone());
                for (probe, channel) in channels {
                    meters.push(FileMeter {
                        reference: i.reference().to_owned(),
                        mode: mode.clone(),
                        probe,
                        channel,
                    });
                }
            }
            _ => {}
        }
    }
    Ok((scopes, meters))
}

/// Default console baud when a `uart` instrument sets none.
const DEFAULT_BAUD: f64 = 115_200.0;

/// The UART consoles and logic analysers `file` lists.
pub(crate) fn digital_bench(file: &CircuitFile) -> Result<of_instruments::digital::DigitalBench, String> {
    use of_instruments::LogicAnalyser;
    use of_instruments::digital::{DigitalBench, UartConsole};
    let mut bench = DigitalBench::default();
    for i in &file.instruments {
        let signals = i
            .probes
            .iter()
            .map(|p| {
                let text = file.probe(p.get_ref()).map_or(p.get_ref().as_str(), |q| q.signal());
                text.parse::<Signal>().map_err(|e| format!("{}: {e}", i.reference()))
            })
            .collect::<Result<Vec<_>, _>>()?;
        let number = |k: &str| match i.params.get(k) {
            Some(of_circuit::ParamValue::Number(x)) => Some(*x),
            _ => None,
        };
        match i.kind() {
            "uart" => {
                let tx = signals
                    .into_iter()
                    .next()
                    .ok_or_else(|| format!("{}: no TX probe", i.reference()))?;
                let baud = number("baud").unwrap_or(DEFAULT_BAUD);
                bench.consoles.push(UartConsole::new(i.reference(), tx, baud));
            }
            "logic" => {
                let la = match of_instruments::profile::lookup(i.profile()) {
                    Some(of_instruments::profile::Profile::Logic(l)) => {
                        LogicAnalyser::real(signals, l, number("sample_rate").unwrap_or(f64::MAX))
                    }
                    _ => LogicAnalyser::ideal(signals),
                }
                .map_err(|e| format!("{}: {e}", i.reference()))?;
                bench.analysers.push((i.reference().to_owned(), la));
            }
            _ => {}
        }
    }
    Ok(bench)
}

fn reached(stop: Stop, at: f64) -> Result<(), String> {
    match stop {
        Stop::Reached(_) => Ok(()),
        other => Err(format!("ESP32-C3 stopped at {:.3} ms ({other:?})", at * 1e3)),
    }
}

impl SessionSim {
    /// Build `kind` from its circuit file and start it.
    pub(crate) fn start(kind: DemoKind, opts: StartOptions) -> Result<Self, String> {
        // Every demo starts on its file's supply unless one is given (a set
        // point that happens to equal the defaults is still the caller's).
        let opts = StartOptions {
            supply: Some(opts.supply.unwrap_or_else(|| file_supply(kind))),
            ..opts
        };
        let s = build_session(kind, opts)?;
        let mut sim = Self::run(kind, s)?;
        sim.position = opts.position.clamp(0.0, 1.0);
        sim.supply = opts.supply.unwrap_or_default().sanitized();
        Ok(sim)
    }

    /// Run an already built `Session` of `kind`.
    pub(crate) fn run(kind: DemoKind, s: Session) -> Result<Self, String> {
        Self::run_inner(kind, s, false)
    }

    /// [`Self::run`] with the GPIO edge log on from the start (parity).
    #[cfg(test)]
    pub(crate) fn run_logged(kind: DemoKind, s: Session) -> Result<Self, String> {
        Self::run_inner(kind, s, true)
    }

    fn run_inner(kind: DemoKind, s: Session, edge_log: bool) -> Result<Self, String> {
        let style = Loop::of(kind);
        let mut meas = MeasurementService::new(SamplePlan::default());
        let led = meas.watch_default(led_probe(kind)?);
        let scope = Scope {
            channel: led,
            scale: 1e3,
            mode: if style == Loop::Pwm {
                ScopeTrace::PeriodAverage {
                    high: trace::PWM_EDGE_HIGH,
                    periods: trace::PWM_AVG_PERIODS,
                    fallback: PWM_AVG_WINDOW,
                }
            } else {
                ScopeTrace::Raw
            },
        };
        let mut meters = MeasurementService::new(SamplePlan::default());
        let meter_ref = meters.watch_default(led_probe(kind)?);
        let mut settled = MeasurementService::new(SamplePlan::default());
        let psu = PsuReadout::watch(&mut settled, "PSU");
        let mut bench = MeasurementService::new(SamplePlan::default());
        let (file_scopes, file_meters) = file_instruments(kind, &mut bench)?;
        let mut d = Self {
            kind,
            s,
            style,
            meas,
            led,
            scope,
            meters,
            meter_ref,
            settled,
            psu,
            next_sample: SimTime::ZERO,
            closed: false,
            bounce: BounceSettings::default(),
            position: 0.5,
            supply: SupplySettings::default(),
            supply_status: SupplyStatus::default(),
            digital: digital_bench(parsed(kind)?)?,
            bench,
            file_scopes,
            file_meters,
        };
        if edge_log && kind.has_mcu() {
            d.cpu().start_edge_log();
        }
        d.record_sample()?;
        if matches!(style, Loop::Blink | Loop::Analog) {
            // The start sample is the grid's t = 0: the grid-safe loop
            // would otherwise take it again.
            d.meas.take_due(d.time());
        }
        if style == Loop::Button {
            d.meas.take_due(d.time());
            d.next_sample = SimTime::from_secs_f64(d.meas.next_due().max(0.0));
        }
        Ok(d)
    }

    pub(crate) fn kind(&self) -> DemoKind {
        self.kind
    }

    fn cpu(&mut self) -> &mut Esp32c3Engine {
        self.s.engine_as::<Esp32c3Engine>("U1").expect("U1 engine")
    }

    /// The series resistor of the LED (`R1` on blink, `R2` elsewhere).
    #[cfg(test)]
    fn resistor_ref(&self) -> &'static str {
        if self.kind == DemoKind::Blink { "R1" } else { "R2" }
    }

    pub(crate) fn time(&self) -> f64 {
        self.s.now().as_secs_f64()
    }

    fn run_to(&mut self, t: SimTime) -> Result<(), String> {
        let Some(step) = self.digital.step() else {
            let stop = self.s.run_until(t).map_err(|e| e.to_string())?;
            return reached(stop, self.time());
        };
        // Consoles and analysers capture on their own finer grid.
        while self.s.now() < t {
            let next = SimTime::from_secs_f64(self.time() + step)
                .min(t)
                .max(SimTime(self.s.now().0 + 1));
            let stop = self.s.run_until(next).map_err(|e| e.to_string())?;
            reached(stop, self.time())?;
            self.digital.capture(&mut self.s).map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// The consoles' titles and text so far.
    pub(crate) fn consoles(&self) -> Vec<crate::digital_ui::ConsoleView> {
        self.digital
            .consoles
            .iter()
            .map(|c| crate::digital_ui::ConsoleView {
                title: format!("{} · {} · {} 8N1", c.reference, c.signal, c.baud.round()),
                text: c.text(),
                framing_errors: c.framing_errors(),
            })
            .collect()
    }

    /// The logic analysers and their captures so far.
    pub(crate) fn logic(&self) -> Vec<(String, of_instruments::LogicAnalyser)> {
        self.digital.analysers.clone()
    }

    /// LED current (mA) from the file's `LED current` probe, now.
    fn instant_ma(&mut self) -> Result<f64, String> {
        let signal = &self.meas.channel(self.led).expect("LED channel").signal;
        Ok(self.s.sample(signal).map_err(|e| e.to_string())? * 1e3)
    }

    /// The LED probe's ring (A).
    #[cfg(test)]
    fn led_ring(&self) -> &trace::TraceBuf {
        &self.meas.channel(self.led).expect("LED channel").ring
    }

    /// The timing capacitor's samples (V) from the file's scope, sampled
    /// with the LED; empty without one.
    fn timing_ramp(&self) -> Vec<(f64, f64)> {
        let signal: Signal = "v(cap)".parse().expect("signal");
        self.bench
            .channels()
            .find(|(_, c)| c.signal == signal)
            .map(|(_, c)| c.ring.iter().collect())
            .unwrap_or_default()
    }

    /// The file's scopes: (reference, [(probe, samples)]).
    pub(crate) fn file_scope_traces(&self) -> ScopeTraces {
        self.file_scopes
            .iter()
            .map(|sc| {
                let chans = sc
                    .channels
                    .iter()
                    .map(|(name, id)| {
                        let pts = self
                            .bench
                            .channel(*id)
                            .map(|c| c.ring.iter().collect())
                            .unwrap_or_default();
                        (name.clone(), pts)
                    })
                    .collect();
                (sc.reference.clone(), chans)
            })
            .collect()
    }

    /// The file's multimeters: (reference, probe, display), each the
    /// average over the LED window (the 555 one-shot) or whole LED periods.
    pub(crate) fn file_meter_readings(&self) -> Vec<(String, String, String)> {
        let led = self.led_ma();
        self.file_meters
            .iter()
            .map(|m| {
                let pts: Vec<(f64, f64)> = self
                    .bench
                    .channel(m.channel)
                    .map(|c| c.ring.iter().collect())
                    .unwrap_or_default();
                let v = trace::average_periods(
                    &pts,
                    &led,
                    self.time(),
                    trace::PWM_EDGE_HIGH,
                    trace::PWM_AVG_PERIODS,
                    trace::LED_AVG_WINDOW,
                );
                let text = match m.mode.as_str() {
                    "dc_a" => format!("{:.2} mA", v * 1e3),
                    _ => format!("{v:.3} V"),
                };
                (m.reference.clone(), m.probe.clone(), text)
            })
            .collect()
    }

    /// The LED probe's samples in mA, as the scope, averages and goldens use them.
    fn led_ma(&self) -> Vec<(f64, f64)> {
        self.scope.samples(&self.meas)
    }

    fn record_sample(&mut self) -> Result<(), String> {
        self.meas.record(&mut self.s).map_err(|e| e.to_string())?;
        if self.kind.file_bench() {
            self.bench.record(&mut self.s).map_err(|e| e.to_string())?;
        }
        self.meters.record(&mut self.s).map_err(|e| e.to_string())?;
        if self.kind.has_mcu() {
            self.cpu().recycle_consumed_edges();
        }
        self.s.commit().map_err(|e| e.to_string())?;
        self.settled.record(&mut self.s).map_err(|e| e.to_string())?;
        self.supply_status = self.psu.status(&mut self.s, self.supply);
        Ok(())
    }

    pub(crate) fn advance(&mut self, seconds: f64) -> Result<(), String> {
        match self.style {
            Loop::Blink | Loop::Analog => self.advance_grid(seconds),
            Loop::Pwm => self.advance_pwm(seconds),
            Loop::Button => self.advance_button(seconds),
        }
    }

    /// Blink and the 555 demos: the engine on the sample grid. A grid instant
    /// the clock does not take yet (the `SimTime` rounding of
    /// [`MeasurementService::next_due`]) gets a 1 ns nudge instead of a
    /// jump to `until`, so an off-grid stop (a frame, a pot or button
    /// change) keeps every later grid instant.
    fn advance_grid(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        while self.s.now() < until {
            let now = self.s.now();
            let grid = SimTime::from_secs_f64(self.meas.next_due().max(0.0));
            if grid <= now {
                if self.meas.take_due(self.time()) {
                    self.record_sample()?;
                } else {
                    self.run_to(now.saturating_add(SimTime::from_ns(1)).min(until))?;
                }
                continue;
            }
            self.run_to(grid.min(until))?;
            if self.meas.take_due(self.time()) {
                self.record_sample()?;
            }
        }
        Ok(())
    }

    fn advance_pwm(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        let origin = self.s.origin();
        while self.s.now() < until {
            let now = self.s.now();
            let grid = SimTime::from_secs_f64(self.meas.next_due().max(0.0));
            let pwm = self
                .cpu()
                .soc
                .bus
                .ledc
                .next_event()
                .map(|t| SimTime(t.0.saturating_sub(origin.0)));
            if grid <= now {
                if self.meas.take_due(self.time()) {
                    self.record_sample()?;
                } else if now < until {
                    let nudge = now.saturating_add(SimTime::from_ns(1)).min(until);
                    self.run_to(nudge)?;
                }
                continue;
            }
            let mut next = grid.min(until);
            if let Some(p) = pwm
                && p > now
                && p < next
            {
                next = p;
            }
            if next > self.s.now() {
                self.run_to(next)?;
            } else if self.s.now() < until {
                self.run_to(until)?;
            }
            let hit_grid = self.meas.take_due(self.time());
            let hit_pwm = pwm.is_some_and(|p| p <= self.s.now());
            if hit_grid || hit_pwm {
                self.record_sample()?;
            }
        }
        Ok(())
    }

    fn advance_button(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        while self.s.now() < until {
            let next = self.next_sample.min(until);
            if next > self.s.now() {
                self.run_to(next)?;
            }
            if self.s.now() >= self.next_sample {
                if self.meas.take_due(self.time()) {
                    self.record_sample()?;
                }
                self.next_sample = SimTime::from_secs_f64(self.meas.next_due().max(0.0));
                if self.next_sample <= self.s.now() {
                    self.next_sample = self.s.now().saturating_add(SimTime::from_ns(1));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn set_scope_window(&mut self, visible: f64) {
        let now = self.time();
        self.meas.set_visible_window(visible, now);
        self.bench.set_visible_window(visible, now);
        self.settled.set_visible_window(visible, now);
        if self.style == Loop::Button {
            self.next_sample = SimTime::from_secs_f64(self.meas.next_due().max(0.0));
        }
    }

    /// Instantaneous LED current (mA).
    pub(crate) fn current_ma(&mut self) -> Result<f64, String> {
        self.instant_ma()
    }

    /// Display-average LED current (mA), averaged as for this kind.
    pub(crate) fn average_ma(&self) -> f64 {
        let pts = self.led_ma();
        match self.style {
            Loop::Blink => trace::average_window(&pts, self.time(), trace::LED_AVG_WINDOW),
            Loop::Pwm => trace::average_periods(
                &pts,
                &pts,
                self.time(),
                trace::PWM_EDGE_HIGH,
                trace::PWM_AVG_PERIODS,
                PWM_AVG_WINDOW,
            ),
            Loop::Button => trace::average_window(&pts, self.time(), BUTTON_AVG_WINDOW),
            // Whole periods of a 555 output when there are any (the PWM
            // dimmer, the astable), else the LED window (the one-shot).
            Loop::Analog => trace::average_periods(
                &pts,
                &pts,
                self.time(),
                trace::PWM_EDGE_HIGH,
                trace::PWM_AVG_PERIODS,
                trace::LED_AVG_WINDOW,
            ),
        }
    }

    /// The scope trace: raw samples, or the PWM demos' moving average.
    pub(crate) fn trace(&self) -> Vec<(f64, f64)> {
        self.scope.trace(&self.meas)
    }

    pub(crate) fn pwm_duty_freq(&self) -> (Option<f64>, Option<f64>) {
        if self.style == Loop::Analog {
            // The 555 demos: whole periods, each output edge timed where the
            // timing capacitor's ramps before and after it meet (the 555
            // flips at the kink), so a 1 ms period on the 50 us grid reads
            // to a fraction of a point.
            let ramp = self.timing_ramp();
            return of_instruments::stats::pwm_duty_freq_ramp(&self.led_ma(), &ramp, self.time(), ANALOG_PWM_WINDOW);
        }
        trace::pwm_duty_freq(&self.led_ma(), self.time(), 0.5)
    }

    pub(crate) fn supply_status(&self) -> SupplyStatus {
        let led = self.led_ma();
        let window = match self.style {
            Loop::Pwm => PWM_AVG_WINDOW,
            Loop::Blink | Loop::Button | Loop::Analog => trace::LED_AVG_WINDOW,
        };
        let mut status = self.supply_status;
        status.amps_avg = Some(self.psu.average_amps(
            &self.settled,
            &led,
            self.time(),
            trace::PWM_EDGE_HIGH,
            trace::PWM_AVG_PERIODS,
            window,
        ));
        status
    }

    pub(crate) fn set_supply(&mut self, settings: SupplySettings) -> Result<(), String> {
        self.supply = settings.sanitized();
        let s = self.supply;
        for (name, value) in [
            ("volts", s.volts),
            ("series_r", s.series_r),
            ("current_limit", s.current_limit),
            ("on", if s.on { 1.0 } else { 0.0 }),
        ] {
            self.s.set_param("PSU", name, value).map_err(|e| e.to_string())?;
        }
        // A short step so the rail updates while playback is paused.
        self.advance(self.time() + 1e-6)?;
        self.record_sample()
    }

    // Dimmer.

    pub(crate) fn position(&self) -> f64 {
        self.position
    }

    pub(crate) fn set_position(&mut self, position: f64) -> Result<(), String> {
        self.position = position.clamp(0.0, 1.0);
        self.s
            .set_param("RV1", "position", self.position)
            .map_err(|e| e.to_string())?;
        // One short step so the wiper voltage updates while playback is paused.
        self.advance(self.time() + 1e-6)
    }

    pub(crate) fn wiper_volts(&mut self) -> Result<f64, String> {
        self.s
            .value("v(gpio2)")
            .ok_or_else(|| "no wiper voltage in the solution".into())
    }

    pub(crate) fn last_adc_uart(&mut self) -> Option<(u32, u32)> {
        let text = String::from_utf8_lossy(&self.uart_bytes().iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned();
        text.lines().rev().find_map(|l| {
            let rest = l.strip_prefix("adc: ")?;
            let (mv, duty) = rest.split_once(" duty: ")?;
            Some((mv.parse().ok()?, duty.parse().ok()?))
        })
    }

    // Button.

    pub(crate) fn closed(&self) -> bool {
        self.closed
    }

    pub(crate) fn set_closed(&mut self, closed: bool) -> Result<(), String> {
        let idle = self.s.pending_timers().next().is_none();
        if self.closed == closed && idle {
            return Ok(());
        }
        self.closed = closed;
        self.s
            .set_param("SW1", "closed", if closed { 1.0 } else { 0.0 })
            .map_err(|e| e.to_string())?;
        self.advance(self.time() + 1e-6)
    }

    pub(crate) fn bounce(&self) -> BounceSettings {
        self.bounce
    }

    pub(crate) fn set_bounce(&mut self, bounce: BounceSettings) -> Result<(), String> {
        self.bounce = bounce;
        self.s
            .set_param("SW1", "bounce", if bounce.enabled { 1.0 } else { 0.0 })
            .map_err(|e| e.to_string())?;
        self.s
            .set_param("SW1", "bounce_seed", bounce.seed as f64)
            .map_err(|e| e.to_string())
    }

    pub(crate) fn button_volts(&mut self) -> Result<f64, String> {
        self.s
            .value("v(gpio9)")
            .ok_or_else(|| "no GPIO9 voltage in the solution".into())
    }

    // UART.

    pub(crate) fn uart_bytes(&mut self) -> &[UartByte] {
        if !self.kind.has_mcu() {
            return &[];
        }
        self.cpu().soc.uart_bytes()
    }

    pub(crate) fn uart_baud(&mut self) -> Option<f64> {
        if !self.kind.has_mcu() {
            return None;
        }
        self.cpu().soc.bus.uart0.baud(APB_HZ)
    }

    pub(crate) fn push_uart_rx(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() && self.kind.has_mcu() {
            self.cpu().soc.push_uart_rx(bytes);
        }
    }

    // Meters (E5c).

    /// Every part's pins and their nets in pin order, for
    /// [`of_instruments::candidates`] (an unconnected pin has net `""`).
    pub(crate) fn layout(&self) -> Vec<PartPins> {
        self.s
            .components()
            .iter()
            .map(|c| {
                let info = c.component.info();
                PartPins {
                    refdes: info.refdes.clone(),
                    pins: info
                        .pins
                        .iter()
                        .map(|p| (p.name.clone(), c.pin_nets.get(&p.name).cloned().unwrap_or_default()))
                        .collect(),
                }
            })
            .collect()
    }

    /// Sample `meter`'s signal from the next sample on.
    pub(crate) fn watch_meter(&mut self, meter: &Multimeter) {
        self.meters.watch_default(meter.signal.clone());
    }

    /// Stop sampling signals that none of `keep` reads.
    pub(crate) fn retain_meters(&mut self, keep: &[Multimeter]) {
        let drop: Vec<ChannelId> = self
            .meters
            .channels()
            .filter(|(id, c)| *id != self.meter_ref && !keep.iter().any(|m| m.signal == c.signal))
            .map(|(id, _)| id)
            .collect();
        for id in drop {
            self.meters.unwatch(id);
        }
    }

    /// Averaging of every meter: whole PWM periods of the LED current, or
    /// the kind's display window when no period is seen (as the PSU readout).
    fn averaging(&self) -> Averaging {
        Averaging {
            reference: self.meter_ref,
            high: trace::PWM_EDGE_HIGH * 1e-3,
            periods: trace::PWM_AVG_PERIODS,
            fallback: match self.style {
                Loop::Pwm => PWM_AVG_WINDOW,
                Loop::Blink | Loop::Button | Loop::Analog => trace::LED_AVG_WINDOW,
            },
        }
    }

    /// `meter`'s reading now; before its first sample (placed while paused)
    /// the instantaneous value.
    pub(crate) fn read_meter(&mut self, meter: &Multimeter) -> Reading {
        let avg = self.averaging();
        match meter.read(&self.meters, &avg, self.time()) {
            Reading::Missing => match self.s.sample(&meter.signal) {
                Ok(v) => match meter.mode {
                    MeterMode::DcVolts => Reading::Volts(v),
                    MeterMode::DcAmps => Reading::Amps {
                        amps: if meter.negate { -v } else { v },
                        part: meter.part.clone().unwrap_or_default(),
                    },
                },
                Err(_) => Reading::Missing,
            },
            r => r,
        }
    }
}

#[cfg(test)]
impl SessionSim {
    /// Raw outputs for the parity goldens.
    pub(crate) fn parity_raw(&mut self) -> crate::parity::Raw {
        let origin = self.s.origin();
        let led = self.led_ma();
        let psu = self.psu.samples_ma(&self.settled);
        let cpu = self.cpu();
        crate::parity::Raw {
            edges: cpu.logged_edges(),
            origin,
            uart: cpu.soc.uart_text(),
            led,
            psu,
        }
    }

    /// Solver and hart counters for the realtime reports.
    #[cfg(all(feature = "gui", test))]
    pub(crate) fn perf_snapshot(&mut self) -> crate::stats::PerfSnapshot {
        let time = self.time();
        let samples = self.led_ring().len();
        crate::stats::PerfSnapshot::from_session(&mut self.s, time, samples)
    }

    /// Capacity of the scope ring.
    #[cfg(all(feature = "gui", test))]
    pub(crate) fn trace_capacity(&self) -> usize {
        self.led_ring().capacity()
    }

    /// The running session (parity steps set raw host params).
    pub(crate) fn session(&mut self) -> &mut Session {
        &mut self.s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parity::{Demo, registry_session, skip_without_rom};
    use crate::supply::SupplyMode;
    use of_instruments::candidates;

    /// The LED series resistor of every demo.
    const R_SERIES: f64 = 330.0;

    const KINDS: [DemoKind; 5] = [
        DemoKind::Blink,
        DemoKind::Dimmer,
        DemoKind::Fade,
        DemoKind::ButtonPoll,
        DemoKind::ButtonIrq,
    ];

    /// What the live worker reads from a running demo.
    #[derive(Debug)]
    struct View {
        time: f64,
        instant: Option<f64>,
        average: f64,
        trace: Vec<(f64, f64)>,
        duty: (Option<f64>, Option<f64>),
        uart: String,
        supply: SupplyStatus,
        /// `v(node)` of the pre-E5c meter nodes.
        volts: Vec<f64>,
        wiper: Option<f64>,
        adc: Option<(u32, u32)>,
        button: Option<f64>,
    }

    fn text(bytes: &[UartByte]) -> String {
        String::from_utf8_lossy(&bytes.iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned()
    }

    fn file_view(s: &mut SessionSim) -> View {
        let pwm = s.style == Loop::Pwm;
        View {
            time: s.time(),
            instant: (!pwm).then(|| s.current_ma().unwrap()),
            average: s.average_ma(),
            trace: s.trace(),
            duty: if s.style == Loop::Button {
                (None, None)
            } else {
                s.pwm_duty_freq()
            },
            uart: text(s.uart_bytes()),
            supply: s.supply_status(),
            volts: old_nodes(s.kind).iter().map(|n| net_volts(s, n)).collect(),
            wiper: (s.kind == DemoKind::Dimmer).then(|| s.wiper_volts().unwrap()),
            adc: if s.kind == DemoKind::Dimmer {
                s.last_adc_uart()
            } else {
                None
            },
            button: s.kind.has_button().then(|| s.button_volts().unwrap()),
        }
    }

    /// The scripted run: 3 ms, with the dimmer's wiper moved and the button
    /// pressed at 1 ms and released at 2 ms.
    fn run_file(kind: DemoKind, idle_draw: bool) -> Option<View> {
        let opts = StartOptions {
            idle_draw,
            ..StartOptions::default()
        };
        let mut s = match SessionSim::start(kind, opts) {
            Ok(s) => s,
            Err(e) if skip_without_rom(&e) => return None,
            Err(e) => panic!("{kind:?}: {e}"),
        };
        s.advance(1e-3).unwrap();
        if kind == DemoKind::Dimmer {
            s.set_position(0.8).unwrap();
        }
        if kind.has_button() {
            s.set_closed(true).unwrap();
        }
        s.advance(2e-3).unwrap();
        if kind.has_button() {
            s.set_closed(false).unwrap();
        }
        s.advance(3e-3).unwrap();
        Some(file_view(&mut s))
    }

    /// The scripted run of [`run_file`] on the demo placed through the model
    /// registry in Rust (`parity::session`; U1's supply current off).
    fn run_registry(kind: DemoKind) -> Option<View> {
        let demo = Demo::ALL.into_iter().find(|d| d.kind() == kind).expect("demo");
        let session = match registry_session(demo) {
            Ok(s) => s,
            Err(e) if skip_without_rom(&e) => return None,
            Err(e) => panic!("{kind:?}: {e}"),
        };
        let mut s = SessionSim::run(kind, session).unwrap();
        s.advance(1e-3).unwrap();
        if kind == DemoKind::Dimmer {
            s.set_position(0.8).unwrap();
        }
        if kind.has_button() {
            s.set_closed(true).unwrap();
        }
        s.advance(2e-3).unwrap();
        if kind.has_button() {
            s.set_closed(false).unwrap();
        }
        s.advance(3e-3).unwrap();
        Some(file_view(&mut s))
    }

    /// RFC 0003's analog rule: `max(floor, 1 ppm)`. `floor` is 1 nA in the
    /// quantity's unit (1e-6 for mA, 1e-9 for A), and 1 nV for volts.
    fn close(a: f64, b: f64, floor: f64) -> bool {
        (a - b).abs() <= floor.max(1e-6 * a.abs().max(b.abs()))
    }

    fn close_opt(a: Option<f64>, b: Option<f64>, floor: f64) -> bool {
        match (a, b) {
            (Some(a), Some(b)) => close(a, b, floor),
            (None, None) => true,
            _ => false,
        }
    }

    const MA: f64 = 1e-6;
    const AMPS: f64 = 1e-9;
    const VOLTS: f64 = 1e-9;

    /// The circuit file and the same circuit placed through the registry in
    /// Rust (both without the ESP32-C3's supply current) show the same LED
    /// current, scope trace, PWM readout, UART text, PSU face and meter
    /// readout (node voltages, branch names and currents), within RFC 0003's
    /// `max(1 nA, 1 ppm)` and with no per-demo exceptions. Since E4d this
    /// replaces the comparison with the deleted hand-wired demos.
    #[test]
    fn every_demo_from_its_file_reads_like_the_registry_built_demo() {
        for kind in KINDS {
            let Some(file) = run_file(kind, false) else {
                continue;
            };
            let hand = run_registry(kind).expect("ROM present for the file run");
            let k = format!("{kind:?}");
            assert_eq!(file.time, hand.time, "{k} time");
            assert!(
                close_opt(file.instant, hand.instant, MA),
                "{k} instant {:?} vs {:?}",
                file.instant,
                hand.instant
            );
            assert!(
                close(file.average, hand.average, MA),
                "{k} average {} vs {}",
                file.average,
                hand.average
            );
            assert_eq!(file.trace.len(), hand.trace.len(), "{k} trace length");
            for (f, h) in file.trace.iter().zip(&hand.trace) {
                assert!(f.0 == h.0 && close(f.1, h.1, MA), "{k} trace {f:?} vs {h:?}");
            }
            assert!(
                close_opt(file.duty.0, hand.duty.0, 1e-9) && close_opt(file.duty.1, hand.duty.1, 1e-9),
                "{k} duty and frequency {:?} vs {:?}",
                file.duty,
                hand.duty
            );
            assert_eq!(file.uart, hand.uart, "{k} UART");
            assert!(
                close(file.supply.volts, hand.supply.volts, VOLTS),
                "{k} PSU volts {} vs {}",
                file.supply.volts,
                hand.supply.volts
            );
            assert!(
                close(file.supply.amps, hand.supply.amps, AMPS),
                "{k} PSU amps {} vs {}",
                file.supply.amps,
                hand.supply.amps
            );
            assert!(
                close_opt(file.supply.amps_avg, hand.supply.amps_avg, AMPS),
                "{k} PSU average {:?} vs {:?}",
                file.supply.amps_avg,
                hand.supply.amps_avg
            );
            assert_eq!(file.supply.mode, hand.supply.mode, "{k} PSU mode");
            assert_eq!(file.supply.outside_rated, hand.supply.outside_rated, "{k} rating");
            assert!(
                close_opt(file.wiper, hand.wiper, VOLTS),
                "{k} wiper {:?} vs {:?}",
                file.wiper,
                hand.wiper
            );
            assert_eq!(file.adc, hand.adc, "{k} ADC line");
            assert!(close_opt(file.button, hand.button, VOLTS), "{k} GPIO9");
            assert_eq!(file.volts.len(), hand.volts.len(), "{k} nodes");
            for (n, (f, h)) in old_nodes(kind).iter().zip(file.volts.iter().zip(&hand.volts)) {
                assert!(close(*f, *h, VOLTS), "{k} node {n}: {f} vs {h}");
            }
        }
    }

    /// The ESP32-C3's 23 mA (blink) moves only the PSU current (by just under
    /// 23 mA, as the rail sits under 3.3 V) and the rail (23 mA × 0.1 Ω).
    #[test]
    fn the_shipped_idle_draw_moves_only_the_psu() {
        let on = run_file(DemoKind::Blink, true).expect("blink needs no ROM");
        let off = run_file(DemoKind::Blink, false).expect("blink needs no ROM");
        let extra = on.supply.amps - off.supply.amps;
        assert!((22.8e-3..23.0e-3).contains(&extra), "idle draw {extra} A");
        let drop = off.supply.volts - on.supply.volts;
        assert!((2.2e-3..2.4e-3).contains(&drop), "rail drop {drop} V");
        assert_eq!(on.uart, off.uart);
        assert!(
            close_opt(on.instant, off.instant, 0.02),
            "{:?} {:?}",
            on.instant,
            off.instant
        );
    }

    #[test]
    fn supply_settings_reach_the_files_bench_psu() {
        let mut s = SessionSim::start(DemoKind::Blink, StartOptions::default()).unwrap();
        s.advance(125e-6).unwrap();
        let i33 = s.current_ma().unwrap();
        assert!((3.0..4.5).contains(&i33), "{i33}");
        s.set_supply(SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        })
        .unwrap();
        let i50 = s.current_ma().unwrap();
        assert!(i50 > i33 * 1.15, "3.3 V {i33} mA, 5 V {i50} mA");
        assert!(s.supply_status().outside_rated);

        let limited = StartOptions {
            supply: Some(SupplySettings {
                current_limit: 0.5e-3,
                ..SupplySettings::default()
            }),
            ..StartOptions::default()
        };
        let mut s = SessionSim::start(DemoKind::Blink, limited).unwrap();
        s.advance(125e-6).unwrap();
        let status = s.supply_status();
        assert_eq!(status.mode, SupplyMode::Cc, "{status:?}");
        assert!(status.amps <= 0.6e-3 + 1e-5, "{status:?}");

        let mut s = SessionSim::start(DemoKind::Blink, StartOptions::default()).unwrap();
        s.set_supply(SupplySettings {
            on: false,
            ..SupplySettings::default()
        })
        .unwrap();
        s.advance(500e-6).unwrap();
        assert_eq!(s.supply_status().mode, SupplyMode::Off);
        assert!(s.supply_status().volts.abs() < 0.05, "{:?}", s.supply_status());
    }

    #[test]
    fn every_demo_names_its_embedded_file() {
        for kind in KINDS {
            let d = demo_file(kind);
            assert_eq!(d.path, format!("circuits/demos/{}.circuit.toml", file_stem(kind)));
            assert!(parsed(kind).is_ok(), "{}", d.path);
        }
    }
    // Ported in E4d from the deleted hand-wired sims (`blink::simulation`,
    // `adc_pwm::PwmSim`, `button::ButtonSim`); same scripts and bounds, on
    // the shipped files.

    const LED_ON_MA: f64 = 1.0;
    const VDD: f64 = 3.3;

    fn start(kind: DemoKind, opts: StartOptions) -> Option<SessionSim> {
        match SessionSim::start(kind, opts) {
            Ok(s) => Some(s),
            Err(e) if skip_without_rom(&e) => None,
            Err(e) => panic!("{kind:?}: {e}"),
        }
    }

    fn dimmer(position: f64) -> Option<SessionSim> {
        start(
            DemoKind::Dimmer,
            StartOptions {
                position,
                ..StartOptions::default()
            },
        )
    }

    fn adc_readings(s: &mut SessionSim) -> Vec<(u32, u32)> {
        text(s.uart_bytes())
            .lines()
            .filter_map(|l| {
                let rest = l.strip_prefix("adc: ")?;
                let (mv, duty) = rest.split_once(" duty: ")?;
                Some((mv.parse().ok()?, duty.parse().ok()?))
            })
            .collect()
    }

    /// `max(1 nA, 1 ppm)`, the strict parity tolerance on currents.
    fn parity_close(a: f64, b: f64) -> bool {
        (a - b).abs() <= f64::max(1e-9, 1e-6 * a.abs().max(b.abs()))
    }

    #[test]
    fn the_led_probe_reads_like_the_old_v_over_r() {
        use of_instruments::{Probeable, Signal};
        // E5b: `i(R…)` through `terminal_currents` against the hand-coded
        // `(v(gpio8) − v(led_a)) / 330` the desktop used until now.
        let mut worst: f64 = 0.0;
        let mut compared = 0;
        for kind in [
            DemoKind::Blink,
            DemoKind::Dimmer,
            DemoKind::Fade,
            DemoKind::ButtonPoll,
            DemoKind::ButtonIrq,
        ] {
            let Some(mut sim) = start(kind, StartOptions::default()) else {
                continue;
            };
            let probe: Signal = format!("i({})", sim.resistor_ref()).parse().unwrap();
            for k in 1..=400 {
                sim.advance(k as f64 * 50e-6).unwrap();
                let gpio = sim.s.value("v(gpio8)").unwrap();
                let anode = sim.s.value("v(led_a)").unwrap();
                let old = (gpio - anode) / R_SERIES;
                let new = sim.s.sample(&probe).unwrap();
                assert!(
                    parity_close(old, new),
                    "{kind:?} t = {}: {old:e} vs {new:e}",
                    sim.time()
                );
                worst = worst.max((old - new).abs());
                compared += 1;
            }
        }
        eprintln!("i(R) vs V/R: {compared} samples, worst |diff| = {worst:e} A");
    }

    /// The pre-E5c meters' probe nodes of `kind` (ground is `0`).
    fn old_nodes(kind: DemoKind) -> Vec<&'static str> {
        let mut nodes = vec!["gpio8", "led_a", "vdd", "0"];
        match kind {
            DemoKind::Dimmer => nodes.push("gpio2"),
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => nodes.push("gpio9"),
            DemoKind::Blink | DemoKind::Fade | DemoKind::Clock555 => {}
            DemoKind::Ne555Astable | DemoKind::Ne555Monostable | DemoKind::Ne555Pwm => {}
        }
        nodes
    }

    /// `v(net)` now.
    fn net_volts(s: &mut SessionSim, net: &str) -> f64 {
        s.s.sample(&Signal::NetVolts(net.to_owned())).unwrap()
    }

    /// The ammeter from `a` toward `b` through `part`, instantaneous (A).
    fn amps_through(s: &mut SessionSim, a: &str, b: &str, part: &str) -> f64 {
        let c = candidates(&s.layout(), a, b)
            .unwrap()
            .into_iter()
            .find(|c| c.refdes == part)
            .unwrap_or_else(|| panic!("{part} does not join {a} and {b}"));
        let v = s.s.sample(&c.signal).unwrap();
        if c.negate { -v } else { v }
    }

    /// Every meter the pre-E5c meters resolved, as recorded by the
    /// old-versus-new check (commit "feat(ui): desktop meters read through
    /// the ideal multimeter", RFC 0004): (demo, a, b, part or "" for a
    /// voltmeter, displayed text, value) after running to 16 ms, the button
    /// closed at 8 ms.
    #[rustfmt::skip]
    const RECORDED: &[(DemoKind, &str, &str, &str, &str, f64)] = &[
        // Blink re-recorded on the grid-safe loop (p3-demos-555): the old
        // loop skipped the grid after its first missed instant and never
        // sampled the LED lit at 0.95 ms.
        (DemoKind::Blink, "gpio8", "led_a", "", "0.650 V", 6.504888994643397e-1),
        (DemoKind::Blink, "led_a", "gpio8", "", "-0.650 V", -6.504888994643397e-1),
        (DemoKind::Blink, "gpio8", "vdd", "", "-1.708 V", -1.7079861554517184e0),
        (DemoKind::Blink, "vdd", "gpio8", "", "1.708 V", 1.7079861554517184e0),
        (DemoKind::Blink, "gpio8", "0", "", "1.590 V", 1.5895184659028772e0),
        (DemoKind::Blink, "0", "gpio8", "", "-1.590 V", -1.5895184659028772e0),
        (DemoKind::Blink, "led_a", "vdd", "", "-2.358 V", -2.3584750549160582e0),
        (DemoKind::Blink, "vdd", "led_a", "", "2.358 V", 2.3584750549160582e0),
        (DemoKind::Blink, "led_a", "0", "", "0.939 V", 9.390295664385366e-1),
        (DemoKind::Blink, "0", "led_a", "", "-0.939 V", -9.390295664385366e-1),
        (DemoKind::Blink, "vdd", "0", "", "3.298 V", 3.297504621354595e0),
        (DemoKind::Blink, "0", "vdd", "", "-3.298 V", -3.297504621354595e0),
        (DemoKind::Blink, "gpio8", "led_a", "R1", "1.97 mA", 1.971178483225272e-3),
        (DemoKind::Blink, "led_a", "gpio8", "R1", "-1.97 mA", -1.971178483225272e-3),
        (DemoKind::Blink, "led_a", "0", "D1", "1.97 mA", 1.9711784832252585e-3),
        (DemoKind::Blink, "0", "led_a", "D1", "-1.97 mA", -1.9711784832252585e-3),
        (DemoKind::Blink, "vdd", "0", "PSU", "-24.95 mA", -2.495378645391588e-2),
        (DemoKind::Blink, "0", "vdd", "PSU", "24.95 mA", 2.495378645391588e-2),
        (DemoKind::Dimmer, "gpio8", "led_a", "", "0.848 V", 8.481949985341278e-1),
        (DemoKind::Dimmer, "led_a", "gpio8", "", "-0.848 V", -8.481949985341278e-1),
        (DemoKind::Dimmer, "gpio8", "vdd", "", "-1.231 V", -1.2312926630970307e0),
        (DemoKind::Dimmer, "vdd", "gpio8", "", "1.231 V", 1.2312926630970307e0),
        (DemoKind::Dimmer, "gpio8", "0", "", "2.066 V", 2.0661191378123167e0),
        (DemoKind::Dimmer, "0", "gpio8", "", "-2.066 V", -2.0661191378123167e0),
        (DemoKind::Dimmer, "gpio8", "gpio2", "", "0.417 V", 4.1741735499032434e-1),
        (DemoKind::Dimmer, "gpio2", "gpio8", "", "-0.417 V", -4.1741735499032434e-1),
        (DemoKind::Dimmer, "led_a", "vdd", "", "-2.079 V", -2.079487661631158e0),
        (DemoKind::Dimmer, "vdd", "led_a", "", "2.079 V", 2.079487661631158e0),
        (DemoKind::Dimmer, "led_a", "0", "", "1.218 V", 1.217924139278187e0),
        (DemoKind::Dimmer, "0", "led_a", "", "-1.218 V", -1.217924139278187e0),
        (DemoKind::Dimmer, "led_a", "gpio2", "", "-0.431 V", -4.307776435438034e-1),
        (DemoKind::Dimmer, "gpio2", "led_a", "", "0.431 V", 4.307776435438034e-1),
        (DemoKind::Dimmer, "vdd", "0", "", "3.297 V", 3.2974118009093463e0),
        (DemoKind::Dimmer, "0", "vdd", "", "-3.297 V", -3.2974118009093463e0),
        (DemoKind::Dimmer, "vdd", "gpio2", "", "1.649 V", 1.648710018087356e0),
        (DemoKind::Dimmer, "gpio2", "vdd", "", "-1.649 V", -1.648710018087356e0),
        (DemoKind::Dimmer, "0", "gpio2", "", "-1.649 V", -1.648701782821991e0),
        (DemoKind::Dimmer, "gpio2", "0", "", "1.649 V", 1.648701782821991e0),
        (DemoKind::Dimmer, "gpio8", "led_a", "R2", "2.57 mA", 2.5702878743458415e-3),
        (DemoKind::Dimmer, "led_a", "gpio8", "R2", "-2.57 mA", -2.5702878743458415e-3),
        (DemoKind::Dimmer, "led_a", "0", "D1", "2.57 mA", 2.5702878743458415e-3),
        (DemoKind::Dimmer, "0", "led_a", "D1", "-2.57 mA", -2.5702878743458415e-3),
        (DemoKind::Dimmer, "vdd", "0", "PSU", "-25.88 mA", -2.588199090653242e-2),
        (DemoKind::Dimmer, "0", "vdd", "PSU", "25.88 mA", 2.588199090653242e-2),
        (DemoKind::Dimmer, "vdd", "gpio2", "RV1", "0.33 mA", 3.2974200361747107e-4),
        (DemoKind::Dimmer, "gpio2", "vdd", "RV1", "-0.33 mA", -3.2974200361747107e-4),
        (DemoKind::Dimmer, "0", "gpio2", "RV1", "-0.33 mA", -3.2974035656439826e-4),
        (DemoKind::Dimmer, "gpio2", "0", "RV1", "0.33 mA", 3.2974035656439826e-4),
        (DemoKind::Fade, "gpio8", "led_a", "", "0.326 V", 3.2588627093071487e-1),
        (DemoKind::Fade, "led_a", "gpio8", "", "-0.326 V", -3.2588627093071487e-1),
        (DemoKind::Fade, "gpio8", "vdd", "", "-2.503 V", -2.5029022260148417e0),
        (DemoKind::Fade, "vdd", "gpio8", "", "2.503 V", 2.5029022260148417e0),
        (DemoKind::Fade, "gpio8", "0", "", "0.795 V", 7.947006912643532e-1),
        (DemoKind::Fade, "0", "gpio8", "", "-0.795 V", -7.947006912643532e-1),
        (DemoKind::Fade, "led_a", "vdd", "", "-2.829 V", -2.8287884969455566e0),
        (DemoKind::Fade, "vdd", "led_a", "", "2.829 V", 2.8287884969455566e0),
        (DemoKind::Fade, "led_a", "0", "", "0.469 V", 4.688144203336383e-1),
        (DemoKind::Fade, "0", "led_a", "", "-0.469 V", -4.688144203336383e-1),
        (DemoKind::Fade, "vdd", "0", "", "3.298 V", 3.2976029172791947e0),
        (DemoKind::Fade, "0", "vdd", "", "-3.298 V", -3.2976029172791947e0),
        (DemoKind::Fade, "gpio8", "led_a", "R2", "0.99 mA", 9.875341543354997e-4),
        (DemoKind::Fade, "led_a", "gpio8", "R2", "-0.99 mA", -9.875341543354997e-4),
        (DemoKind::Fade, "led_a", "0", "D1", "0.99 mA", 9.875341543354997e-4),
        (DemoKind::Fade, "0", "led_a", "D1", "-0.99 mA", -9.875341543354997e-4),
        (DemoKind::Fade, "vdd", "0", "PSU", "-23.97 mA", -2.3970827208051357e-2),
        (DemoKind::Fade, "0", "vdd", "PSU", "23.97 mA", 2.3970827208051357e-2),
        (DemoKind::ButtonPoll, "gpio8", "led_a", "", "1.305 V", 1.3050330761452755e0),
        (DemoKind::ButtonPoll, "led_a", "gpio8", "", "-1.305 V", -1.3050330761452755e0),
        (DemoKind::ButtonPoll, "gpio8", "vdd", "", "-0.119 V", -1.1863937065401983e-1),
        (DemoKind::ButtonPoll, "vdd", "gpio8", "", "0.119 V", 1.1863937065401983e-1),
        (DemoKind::ButtonPoll, "gpio8", "0", "", "3.179 V", 3.178659719914133e0),
        (DemoKind::ButtonPoll, "0", "gpio8", "", "-3.179 V", -3.178659719914133e0),
        (DemoKind::ButtonPoll, "gpio8", "gpio9", "", "3.179 V", 3.178652392599102e0),
        (DemoKind::ButtonPoll, "gpio9", "gpio8", "", "-3.179 V", -3.178652392599102e0),
        (DemoKind::ButtonPoll, "led_a", "vdd", "", "-1.424 V", -1.4236724467992947e0),
        (DemoKind::ButtonPoll, "vdd", "led_a", "", "1.424 V", 1.4236724467992947e0),
        (DemoKind::ButtonPoll, "led_a", "0", "", "1.874 V", 1.8736266437688562e0),
        (DemoKind::ButtonPoll, "0", "led_a", "", "-1.874 V", -1.8736266437688562e0),
        (DemoKind::ButtonPoll, "led_a", "gpio9", "", "1.874 V", 1.873619316453828e0),
        (DemoKind::ButtonPoll, "gpio9", "led_a", "", "-1.874 V", -1.873619316453828e0),
        (DemoKind::ButtonPoll, "vdd", "0", "", "3.297 V", 3.2972990905681527e0),
        (DemoKind::ButtonPoll, "0", "vdd", "", "-3.297 V", -3.2972990905681527e0),
        (DemoKind::ButtonPoll, "vdd", "gpio9", "", "3.297 V", 3.297291763253124e0),
        (DemoKind::ButtonPoll, "gpio9", "vdd", "", "-3.297 V", -3.297291763253124e0),
        (DemoKind::ButtonPoll, "0", "gpio9", "", "-0.000 V", -7.327315029450649e-6),
        (DemoKind::ButtonPoll, "gpio9", "0", "", "0.000 V", 7.327315029450649e-6),
        (DemoKind::ButtonPoll, "gpio8", "led_a", "R2", "3.95 mA", 3.95464568528871e-3),
        (DemoKind::ButtonPoll, "led_a", "gpio8", "R2", "-3.95 mA", -3.95464568528871e-3),
        (DemoKind::ButtonPoll, "led_a", "0", "D1", "3.95 mA", 3.95464568528871e-3),
        (DemoKind::ButtonPoll, "0", "led_a", "D1", "-3.95 mA", -3.95464568528871e-3),
        (DemoKind::ButtonPoll, "vdd", "0", "PSU", "-27.01 mA", -2.700909431847977e-2),
        (DemoKind::ButtonPoll, "0", "vdd", "PSU", "27.01 mA", 2.700909431847977e-2),
        (DemoKind::ButtonPoll, "0", "gpio9", "SW1", "-0.07 mA", -7.327315029450646e-5),
        (DemoKind::ButtonPoll, "gpio9", "0", "SW1", "0.07 mA", 7.327315029450646e-5),
        (DemoKind::ButtonIrq, "gpio8", "led_a", "", "1.305 V", 1.3045465275404955e0),
        (DemoKind::ButtonIrq, "led_a", "gpio8", "", "-1.305 V", -1.3045465275404955e0),
        (DemoKind::ButtonIrq, "gpio8", "vdd", "", "-0.119 V", -1.1859513896269996e-1),
        (DemoKind::ButtonIrq, "vdd", "gpio8", "", "0.119 V", 1.1859513896269996e-1),
        (DemoKind::ButtonIrq, "gpio8", "0", "", "3.179 V", 3.1794031855478737e0),
        (DemoKind::ButtonIrq, "0", "gpio8", "", "-3.179 V", -3.1794031855478737e0),
        (DemoKind::ButtonIrq, "gpio8", "gpio9", "", "3.179 V", 3.1793958566789944e0),
        (DemoKind::ButtonIrq, "gpio9", "gpio8", "", "-3.179 V", -3.1793958566789944e0),
        (DemoKind::ButtonIrq, "led_a", "vdd", "", "-1.423 V", -1.4231416665031953e0),
        (DemoKind::ButtonIrq, "vdd", "led_a", "", "1.423 V", 1.4231416665031953e0),
        (DemoKind::ButtonIrq, "led_a", "0", "", "1.875 V", 1.8748566580073784e0),
        (DemoKind::ButtonIrq, "0", "led_a", "", "-1.875 V", -1.8748566580073784e0),
        (DemoKind::ButtonIrq, "led_a", "gpio9", "", "1.875 V", 1.8748493291384993e0),
        (DemoKind::ButtonIrq, "gpio9", "led_a", "", "-1.875 V", -1.8748493291384993e0),
        (DemoKind::ButtonIrq, "vdd", "0", "", "3.298 V", 3.2979983245105733e0),
        (DemoKind::ButtonIrq, "0", "vdd", "", "-3.298 V", -3.2979983245105733e0),
        (DemoKind::ButtonIrq, "vdd", "gpio9", "", "3.298 V", 3.297990995641695e0),
        (DemoKind::ButtonIrq, "gpio9", "vdd", "", "-3.298 V", -3.297990995641695e0),
        (DemoKind::ButtonIrq, "0", "gpio9", "", "-0.000 V", -7.328868879203033e-6),
        (DemoKind::ButtonIrq, "gpio9", "0", "", "0.000 V", 7.328868879203033e-6),
        (DemoKind::ButtonIrq, "gpio8", "led_a", "R2", "3.95 mA", 3.95317129557726e-3),
        (DemoKind::ButtonIrq, "led_a", "gpio8", "R2", "-3.95 mA", -3.95317129557726e-3),
        (DemoKind::ButtonIrq, "led_a", "0", "D1", "3.95 mA", 3.95317129557726e-3),
        (DemoKind::ButtonIrq, "0", "led_a", "D1", "-3.95 mA", -3.95317129557726e-3),
        (DemoKind::ButtonIrq, "vdd", "0", "PSU", "-20.02 mA", -2.0016754894261343e-2),
        (DemoKind::ButtonIrq, "0", "vdd", "PSU", "20.02 mA", 2.0016754894261343e-2),
        (DemoKind::ButtonIrq, "0", "gpio9", "SW1", "-0.07 mA", -7.328868879203031e-5),
        (DemoKind::ButtonIrq, "gpio9", "0", "SW1", "0.07 mA", 7.328868879203031e-5),
    ];

    /// E5c: the multimeter still reads the values the pre-E5c meters read
    /// (period-averaged as the multimeter averages; PSU as the old PSU
    /// readout): displayed strings equal, values within max(1 nA, 1 ppm)
    /// (1 nV for volts), on all five demos.
    #[test]
    fn the_meters_read_the_recorded_values_on_all_demos() {
        let close = |x: f64, y: f64| (x - y).abs() <= 1e-9_f64.max(1e-6 * x.abs().max(y.abs()));
        let mut checked = 0;
        for kind in KINDS {
            let Some(mut s) = start(kind, StartOptions::default()) else {
                continue;
            };
            let layout = s.layout();
            let rows: Vec<_> = RECORDED.iter().filter(|r| r.0 == kind).collect();
            assert!(!rows.is_empty(), "{kind:?} has recorded meters");
            let meters: Vec<Multimeter> = rows
                .iter()
                .map(|&&(_, a, b, part, _, _)| {
                    if part.is_empty() {
                        Multimeter::voltmeter(a, b)
                    } else {
                        let c = candidates(&layout, a, b)
                            .unwrap()
                            .into_iter()
                            .find(|c| c.refdes == part);
                        Multimeter::ammeter(&c.unwrap_or_else(|| panic!("{kind:?} {part} joins {a} and {b}")))
                    }
                })
                .collect();
            for m in &meters {
                s.watch_meter(m);
            }
            if matches!(kind, DemoKind::ButtonPoll | DemoKind::ButtonIrq) {
                s.advance(8e-3).unwrap();
                s.set_closed(true).unwrap();
            }
            s.advance(16e-3).unwrap();
            for (&&(_, a, b, part, text, value), m) in rows.iter().zip(&meters) {
                let r = s.read_meter(m);
                let v = match &r {
                    Reading::Volts(v) | Reading::Amps { amps: v, .. } => *v,
                    other => panic!("{kind:?} {a}->{b} {part}: {other:?}"),
                };
                assert_eq!(r.display(), text, "{kind:?} {a}->{b} {part} display");
                assert!(close(v, value), "{kind:?} {a}->{b} {part}: {v:e} vs recorded {value:e}");
                checked += 1;
            }
        }
        eprintln!("{checked} recorded meter readings matched");
    }
    /// The PSU face and LED average before E5d (`psu_samples` ring and the
    /// desktop's own sample clock), recorded on 31ff9f6: (demo, current
    /// limit if lowered, time, volts, amps, averaged amps, mode, outside
    /// rated, `average_ma`). The button demos close SW1 at 8 ms.
    #[rustfmt::skip]
    #[allow(clippy::type_complexity)]
    const FACES: &[(DemoKind, Option<f64>, f64, f64, f64, f64, SupplyMode, bool, f64)] = &[
        // Re-pinned on the grid-safe loop (p3-demos-555): the old loop had
        // not sampled the LED lit at 0.95 ms by 12 ms (average 22.98 mA,
        // LED 3.29e-12 mA).
        (DemoKind::Blink, None, 1.2e-2, 3.297701601913488e0, 2.298398086511888e-2, 2.4953786285306433e-2, SupplyMode::Cv, false, 1.9711784422092875e0),
        (DemoKind::Blink, None, 1.6e-2, 3.297701601913488e0, 2.298398086511888e-2, 2.4953786453914883e-2, SupplyMode::Cv, false, 1.9711783577801563e0),
        (DemoKind::Dimmer, None, 1.2e-2, 3.297273011213674e0, 2.7269887863257125e-2, 2.5881843413710973e-2, SupplyMode::Cv, false, 2.5701402772434303e0),
        (DemoKind::Dimmer, None, 1.6e-2, 3.2972730014380813e0, 2.7269985619184922e-2, 2.588199090653242e-2, SupplyMode::Cv, false, 2.570287874345841e0),
        (DemoKind::Fade, None, 1.2e-2, 3.297701601913488e0, 2.298398086511888e-2, 2.397275294302185e-2, SupplyMode::Cv, false, 9.894612498139179e-1),
        (DemoKind::Fade, None, 1.6e-2, 3.297701601913488e0, 2.298398086511888e-2, 2.3970827208051357e-2, SupplyMode::Cv, false, 9.875341543354996e-1),
        (DemoKind::ButtonPoll, None, 1.2e-2, 3.297299090568152e0, 2.7009094318479754e-2, 2.700909431847977e-2, SupplyMode::Cv, false, 3.954645685288708e0),
        (DemoKind::ButtonPoll, None, 1.6e-2, 3.297299090568152e0, 2.7009094318479754e-2, 2.700909431847977e-2, SupplyMode::Cv, false, 3.954645685288709e0),
        (DemoKind::ButtonIrq, None, 1.2e-2, 3.297998473867777e0, 2.0015261322230415e-2, 2.0014715136282488e-2, SupplyMode::Cv, false, 3.951130544092355e0),
        (DemoKind::ButtonIrq, None, 1.6e-2, 3.2979982788436564e0, 2.00172115634345e-2, 2.0016754894261343e-2, SupplyMode::Cv, false, 3.953171295577258e0),
        (DemoKind::Blink, Some(0.002), 1.2e-2, 2.8695652165678637e-1, 2e-3, 2e-3, SupplyMode::Cc, true, 2.812003567523824e-4),
        (DemoKind::Blink, Some(0.002), 1.6e-2, 2.8695652165678637e-1, 2e-3, 2e-3, SupplyMode::Cc, true, 2.8161368386905313e-4),
        (DemoKind::Dimmer, Some(0.005), 1.2e-2, 7.072438667498492e-1, 5e-3, 5e-3, SupplyMode::Cc, true, 2.4933372214307668e-5),
        (DemoKind::Dimmer, Some(0.005), 1.6e-2, 7.072438667498492e-1, 5e-3, 5e-3, SupplyMode::Cc, true, 1.8216836455929914e-5),
    ];

    /// E5d: the PSU face on the measurement service and the service's grid
    /// read what the desktop read before, on all five demos and in CC
    /// (max(1 nA, 1 ppm); 1 nV; mode and rating exact).
    #[test]
    fn the_supply_face_and_led_average_are_unchanged_by_e5d() {
        let close = |x: f64, y: f64| (x - y).abs() <= 1e-9_f64.max(1e-6 * x.abs().max(y.abs()));
        let mut checked = 0;
        let mut worst: f64 = 0.0;
        let mut last: Option<(DemoKind, Option<f64>, SessionSim)> = None;
        for &(kind, limit, t, volts, amps, avg, mode, outside, led) in FACES {
            let same = last.as_ref().is_some_and(|(k, l, _)| *k == kind && *l == limit);
            if !same {
                let Some(mut s) = start(kind, StartOptions::default()) else {
                    last = None;
                    continue;
                };
                if let Some(l) = limit {
                    s.set_supply(SupplySettings {
                        current_limit: l,
                        ..SupplySettings::default()
                    })
                    .unwrap();
                }
                if matches!(kind, DemoKind::ButtonPoll | DemoKind::ButtonIrq) {
                    s.advance(8e-3).unwrap();
                    s.set_closed(true).unwrap();
                }
                last = Some((kind, limit, s));
            }
            let Some((_, _, s)) = last.as_mut() else { continue };
            s.advance(t).unwrap();
            let st = s.supply_status();
            let got = st.amps_avg.unwrap();
            let at = format!("{kind:?} {limit:?} at {t}");
            assert!(close(st.volts, volts), "{at}: volts {} vs {volts}", st.volts);
            assert!(close(st.amps, amps), "{at}: amps {} vs {amps}", st.amps);
            assert!(close(got, avg), "{at}: average {got} vs {avg}");
            assert_eq!((st.mode, st.outside_rated), (mode, outside), "{at}");
            // average_ma is in mA: 1 nA is 1e-6 mA.
            let ma = s.average_ma();
            assert!(
                (ma - led).abs() <= 1e-6_f64.max(1e-6 * led.abs()),
                "{at}: LED {ma} vs {led} mA"
            );
            worst = worst
                .max((got - avg).abs())
                .max((st.amps - amps).abs())
                .max((ma - led).abs() * 1e-3);
            checked += 1;
        }
        eprintln!("{checked} faces matched; worst current difference {worst:e} A");
    }
    #[test]
    fn the_led_current_agrees_with_its_series_resistor() {
        use of_instruments::{Probeable, Signal};
        // E5c: the solver's diode current `i(D1)` against the series-R current,
        // at max(1 nA, 1 ppm), on every demo, at the scope's sample instants.
        let mut worst: f64 = 0.0;
        let mut compared = 0;
        for kind in [
            DemoKind::Blink,
            DemoKind::Dimmer,
            DemoKind::Fade,
            DemoKind::ButtonPoll,
            DemoKind::ButtonIrq,
        ] {
            let Some(mut sim) = start(kind, StartOptions::default()) else {
                continue;
            };
            let r: Signal = format!("i({})", sim.resistor_ref()).parse().unwrap();
            let d: Signal = "i(D1)".parse().unwrap();
            for k in 1..=400 {
                sim.advance(k as f64 * 50e-6).unwrap();
                let ir = sim.s.sample(&r).unwrap();
                let id = sim.s.sample(&d).unwrap();
                assert!(
                    parity_close(ir, id),
                    "{kind:?} t = {}: R {ir:e} vs D1 {id:e}",
                    sim.time()
                );
                worst = worst.max((ir - id).abs());
                compared += 1;
            }
        }
        eprintln!("i(D1) vs i(R): {compared} samples, worst |diff| = {worst:e} A");
    }

    #[test]
    fn the_session_resolves_every_signal_kind() {
        use of_instruments::{ProbeError, Probeable, Signal};
        let Some(mut sim) = start(DemoKind::Blink, StartOptions::default()) else {
            return;
        };
        sim.advance(1e-3).unwrap();
        let s = &mut sim.s;
        let mut get = |x: &str| s.sample(&x.parse::<Signal>().unwrap());
        let vdd = get("v(vdd)").unwrap();
        assert!((3.0..3.4).contains(&vdd), "{vdd}");
        assert_eq!(get("v(gnd)").unwrap(), 0.0);
        assert_eq!(get("v(0)").unwrap(), 0.0);
        assert_eq!(get("v(vdd,gnd)").unwrap(), vdd);
        let i = get("i(R1)").unwrap();
        assert_eq!(get("i(R1.1)").unwrap(), i);
        assert_eq!(get("i(R1.2)").unwrap(), -i);
        let psu = get("r(PSU.volts)").unwrap();
        assert!((psu - vdd).abs() < 0.1, "{psu} vs {vdd}");
        let unresolved = |x: &str| ProbeError::Unresolved(x.parse().unwrap());
        assert_eq!(get("v(nowhere)"), Err(unresolved("v(nowhere)")));
        assert_eq!(get("i(R9)"), Err(unresolved("i(R9)")));
        assert_eq!(get("i(R1.3)"), Err(unresolved("i(R1.3)")));
        assert_eq!(get("i(U1)"), Err(unresolved("i(U1)")), "U1 has more than two pins");
        assert_eq!(get("r(PSU.nope)"), Err(unresolved("r(PSU.nope)")));
        // E5g: d(REF.pin) is the pin's net voltage through the ideal input.
        let d = get("d(U1.GPIO8)").unwrap();
        assert_eq!(get("d(U1.NOPE)"), Err(unresolved("d(U1.NOPE)")));
        let pin = s.pin_volts("U1", "GPIO8").unwrap();
        let level = if pin > of_instruments::logic::IDEAL_THRESHOLD {
            1.0
        } else {
            0.0
        };
        assert_eq!(d, level, "{pin} V");
        assert!((s.now_secs() - 1e-3).abs() < 1e-12);
    }

    #[test]
    fn blink_advances_firmware_and_records_led_current() {
        let end = 2.6e-3;
        let mut s = start(DemoKind::Blink, StartOptions::default()).expect("blink needs no ROM");
        assert_eq!(s.time(), 0.0);
        assert!(s.uart_bytes().is_empty());
        s.advance(125e-6).unwrap();
        assert!((3.0..4.5).contains(&s.current_ma().unwrap()));
        assert_eq!(text(s.uart_bytes()), "ohmforge blinky\n");
        s.advance(375e-6).unwrap();
        assert!(s.current_ma().unwrap().abs() < 1e-3);
        s.advance(625e-6).unwrap();
        assert!((3.0..4.5).contains(&s.current_ma().unwrap()));
        s.advance(end).unwrap();
        let trace = s.trace();
        assert!(trace.iter().any(|&(_, ma)| ma > LED_ON_MA), "LED never on: {trace:?}");
        assert!(trace.iter().any(|&(_, ma)| ma.abs() < 1e-3), "LED never off: {trace:?}");
        assert!((trace.last().unwrap().0 - end).abs() < 1e-9);
    }

    #[cfg(feature = "gui")]
    #[test]
    fn monitor_receives_blink_banner_when_synced() {
        let mut monitor = of_instruments::SerialMonitor::new();
        let mut s = start(DemoKind::Blink, StartOptions::default()).expect("blink needs no ROM");
        s.advance(125e-6).unwrap();
        for b in s.uart_bytes() {
            monitor.push_rx(b.at, &[b.byte]);
        }
        assert_eq!(monitor.text_device_to_host(), "ohmforge blinky\n");
    }

    #[test]
    fn push_uart_rx_is_readable_from_fifo() {
        let mut s = start(DemoKind::Blink, StartOptions::default()).expect("blink needs no ROM");
        s.push_uart_rx(b"Z");
        // 8N1 at a typical baud is ~87 µs per byte; wait a couple of frames.
        let baud = s.uart_baud().unwrap_or(115_200.0).max(1.0);
        s.advance(s.time() + 20.0 / baud).unwrap();
        assert_eq!(s.cpu().soc.bus.uart0.pop_rx(), b'Z');
    }

    #[test]
    fn tiny_advances_keep_the_scope_until_the_left_edge() {
        let mut s = start(DemoKind::Blink, StartOptions::default()).expect("blink needs no ROM");
        let visible = crate::trace::DEFAULT_VISIBLE_WINDOW;
        // Many slices much smaller than SAMPLE_STEP, as at a very low playback speed.
        let (end, dt) = (0.20, 5e-6);
        let mut t = s.time();
        while t < end {
            t += dt;
            s.advance(t).unwrap();
        }
        let pts = s.trace();
        let left = s.time() - visible;
        assert!(!pts.is_empty(), "empty trace");
        assert!(
            pts[0].0 <= left + crate::trace::SAMPLE_STEP * 2.0,
            "cleared too early: first {} left {left}",
            pts[0].0
        );
        let on_screen = pts.iter().filter(|(tt, _)| *tt >= left - 1e-12).count();
        assert!(on_screen > 100, "sparse on-screen trace: {on_screen}");
    }

    #[test]
    fn dimmer_reads_the_pot_and_the_slider_changes_the_duty() {
        let Some(mut s) = dimmer(0.3) else { return };
        assert!((s.wiper_volts().unwrap() - 0.3 * VDD).abs() < 0.05);
        s.advance(15e-3).unwrap();
        let mid = adc_readings(&mut s);
        assert!(
            mid.iter()
                .any(|&(mv, _)| (f64::from(mv) - 0.3 * VDD * 1e3).abs() < 80.0),
            "{mid:?}"
        );
        s.set_position(0.85).unwrap();
        let after_turn = adc_readings(&mut s).len();
        s.advance(20e-3).unwrap();
        let high = adc_readings(&mut s);
        assert!(high.len() > after_turn, "{high:?}");
        assert!(
            high.iter()
                .any(|&(mv, duty)| (f64::from(mv) - 0.85 * VDD * 1e3).abs() < 80.0 && duty > 800),
            "{high:?}"
        );
        assert!(s.average_ma() > 1.0, "average {} mA", s.average_ma());
        assert!(s.trace().len() > 10);
    }

    #[test]
    fn fade_ramps_the_display_average() {
        let Some(mut s) = start(DemoKind::Fade, StartOptions::default()) else {
            return;
        };
        s.advance(20e-3).unwrap();
        let early = s.average_ma();
        s.advance(40e-3).unwrap();
        let later = s.average_ma();
        // The hardware fade climbs for ~51 ms; 20 ms to 60 ms should get brighter.
        assert!(later > early + 0.3, "early {early} mA, later {later} mA");
        let t = text(s.uart_bytes());
        assert!(t.contains("ohmforge esp-hal led fade"), "{t:?}");
    }

    #[test]
    fn pot_divider_scales_with_supply_voltage() {
        let Some(mut s) = dimmer(0.5) else { return };
        s.set_position(0.5).unwrap();
        // Allow one ADC print to catch up; the wiper itself updates immediately.
        let mid = s.wiper_volts().unwrap();
        assert!((mid - 0.5 * VDD).abs() < 0.05, "wiper at 3.3 V: {mid}");
        s.set_supply(SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        })
        .unwrap();
        let high = s.wiper_volts().unwrap();
        assert!(
            (high - 0.5 * 5.0).abs() < 0.08,
            "wiper should scale with the rail: {high}"
        );
        assert!(s.supply_status().outside_rated);
    }

    #[test]
    fn current_limit_clamps_dimmer_led_current() {
        let opts = StartOptions {
            supply: Some(SupplySettings {
                current_limit: 0.5e-3,
                ..SupplySettings::default()
            }),
            position: 1.0,
            ..StartOptions::default()
        };
        let Some(mut s) = start(DemoKind::Dimmer, opts) else {
            return;
        };
        s.advance(20e-3).unwrap();
        let status = s.supply_status();
        let avg = s.average_ma();
        assert!(
            status.mode == SupplyMode::Cc || avg < 1.0,
            "limit did not clamp: status={status:?} avg={avg} mA"
        );
        assert!(status.amps <= 0.7e-3, "{status:?}");
    }

    #[test]
    fn dimmer_averaged_led_current_is_steady_at_constant_duty() {
        let Some(mut s) = dimmer(0.5) else { return };
        s.advance(25e-3).unwrap();
        let readings = adc_readings(&mut s);
        assert!(
            readings.iter().any(|&(_, duty)| duty > 0),
            "no UART duty print: {readings:?}"
        );
        let late: Vec<f64> = s
            .trace()
            .iter()
            .filter(|(t, _)| *t >= s.time() - 8e-3)
            .map(|(_, i)| *i)
            .collect();
        assert!(late.len() >= 8, "not enough averaged samples: {}", late.len());
        let mean = late.iter().sum::<f64>() / late.len() as f64;
        let peak = late.iter().fold(0.0_f64, |m, v| m.max((v - mean).abs()));
        assert!(mean > 1.5, "expected a mid-duty LED average, got {mean} mA");
        assert!(peak < 0.08, "ripple {peak} mA around {mean} mA at constant duty");
        let vdd = net_volts(&mut s, "vdd");
        let i_pot = vdd / 10_000.0;
        // The ESP32-C3's own supply current (`quiescent_a`, sized for 3.3 V).
        let i_chip = vdd * of_soc_esp32c3::QUIESCENT_A / 3.3;
        let i_psu = s.supply_status().display_amps();
        let i_led = s.average_ma() * 1e-3;
        assert!(
            (i_psu - (i_led + i_pot + i_chip)).abs() < 2e-4,
            "PSU {i_psu} A should be LED {i_led} A plus pot {i_pot} A plus ESP32-C3 {i_chip} A"
        );
    }

    #[test]
    fn polling_copies_the_button_to_the_led() {
        let Some(mut s) = start(DemoKind::ButtonPoll, StartOptions::default()) else {
            return;
        };
        s.advance(200e-6).unwrap();
        assert!(s.average_ma() < 0.2, "LED should be off at rest: {} mA", s.average_ma());
        let v_up = s.button_volts().unwrap();
        assert!(v_up > 3.0, "pull-up should hold GPIO9 high: {v_up}");
        s.set_closed(true).unwrap();
        s.advance(s.time() + 2e-3).unwrap();
        let inst = s.current_ma().unwrap();
        let v_down = s.button_volts().unwrap();
        assert!(inst > 1.0, "LED should be on while held: {inst} mA, GPIO9 {v_down} V");
        assert!(v_down < 0.1, "closed button should pull GPIO9 down: {v_down}");
        // Default bounce is the canonical press train (5 edges), ending low.
        let t = text(s.uart_bytes());
        assert!(t.contains("poll: 5 low"), "{t:?}");
    }

    #[test]
    fn bounce_can_be_turned_off() {
        let Some(mut s) = start(DemoKind::ButtonPoll, StartOptions::default()) else {
            return;
        };
        s.set_bounce(BounceSettings {
            enabled: false,
            seed: 0,
        })
        .unwrap();
        s.advance(200e-6).unwrap();
        s.set_closed(true).unwrap();
        s.advance(s.time() + 2e-3).unwrap();
        let t = text(s.uart_bytes());
        assert!(t.contains("poll: 1 low"), "{t:?}");
        assert!(!t.contains("poll: 5 low"), "{t:?}");
    }

    // Ported from `meters` (they ran on the hand-wired blink and dimmer).

    #[test]
    fn blink_led_series_current_matches_the_resistor_drop() {
        let mut s = start(DemoKind::Blink, StartOptions::default()).expect("blink needs no ROM");
        s.advance(125e-6).unwrap();
        let i_r = amps_through(&mut s, "gpio8", "led_a", "R1");
        let i_d = amps_through(&mut s, "led_a", "0", "D1");
        let analytic = (net_volts(&mut s, "gpio8") - net_volts(&mut s, "led_a")) / R_SERIES;
        assert!((i_r - analytic).abs() < 1e-9, "{i_r} vs {analytic}");
        assert!((i_d - analytic).abs() < 1e-9, "{i_d} vs {analytic}");
        assert!((3.0..4.5).contains(&(analytic * 1e3)), "{} mA", analytic * 1e3);
        // Same current, opposite sign, if the leads are swapped.
        assert!((amps_through(&mut s, "led_a", "gpio8", "R1") + analytic).abs() < 1e-9);
    }

    #[test]
    fn dimmer_pot_divider_voltage_matches_the_wiper() {
        let Some(mut s) = dimmer(0.5) else { return };
        let vdd = net_volts(&mut s, "vdd");
        let wiper = net_volts(&mut s, "gpio2");
        assert!((wiper - 0.5 * vdd).abs() < 0.05, "wiper {wiper} V, VDD {vdd} V");
        // The two pot legs read their own segment and their currents match.
        let i_a = amps_through(&mut s, "0", "gpio2", "RV1");
        let i_b = amps_through(&mut s, "vdd", "gpio2", "RV1");
        assert!((i_a + i_b).abs() < 5e-6, "unloaded wiper: {i_a} A vs {i_b} A");
        s.set_supply(SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        })
        .unwrap();
        let wiper = net_volts(&mut s, "gpio2");
        assert!(
            (wiper - 0.5 * net_volts(&mut s, "vdd")).abs() < 0.08,
            "wiper should scale with the rail: {wiper}"
        );
    }
    /// Changing the supply at t = 0, before the demo has advanced, keeps the
    /// rail at or under the new set point and the wiper under the rail at
    /// every step (it once spiked to 70 V for one step).
    #[test]
    fn a_supply_change_before_the_first_step_never_overshoots() {
        let Some(mut s) = dimmer(0.5) else { return };
        s.set_supply(SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        })
        .unwrap();
        let mut t = s.time();
        for _ in 0..200 {
            let v = net_volts(&mut s, "vdd");
            let w = s.wiper_volts().unwrap();
            assert!(v <= 5.0 + 1e-3, "rail {v} V at {t} s");
            assert!(w <= v + 1e-6, "wiper {w} V above the rail {v} V at {t} s");
            assert_ne!(s.supply_status().mode, SupplyMode::Cc, "at {t} s");
            t += 1e-6;
            s.advance(t).unwrap();
        }
        assert!((s.wiper_volts().unwrap() - 2.5).abs() < 0.08);
    }
}

#[cfg(test)]
mod digital_tests {
    use super::*;

    #[test]
    fn the_bench_lists_the_files_consoles_and_analysers() {
        let demo = demo_file(DemoKind::Blink).text;
        let text = format!(
            "{demo}\n[[instrument]]\nref = \"XU1\"\nkind = \"uart\"\nprobes = [\"d(U1.GPIO8)\"]\nparams = {{ baud = 9600 }}\n\n[[instrument]]\nref = \"XLA1\"\nkind = \"logic\"\nprofile = \"saleae.logic8\"\nprobes = [\"d(U1.GPIO8)\"]\n"
        );
        let file = of_circuit::parse(&text).unwrap();
        let bench = digital_bench(&file).unwrap();
        assert_eq!(bench.consoles.len(), 1);
        assert_eq!(bench.consoles[0].baud, 9600.0);
        // One channel: the Saleae's 100 MS/s.
        assert_eq!(bench.analysers[0].1.period, Some(1e-8));
        assert_eq!(bench.step(), Some(of_instruments::digital::MIN_LOGIC_STEP));
        // The other shipped demos list none, so their runs are unchanged; the
        // clock counter reads its TX on a console and a logic analyser.
        for k in KINDS {
            let bench = digital_bench(parsed(k).unwrap()).unwrap();
            if k == DemoKind::Clock555 {
                assert_eq!(bench.consoles.len(), 1);
                assert_eq!(bench.consoles[0].baud, 28818.0);
                assert_eq!(bench.analysers.len(), 1);
            } else {
                assert!(bench.is_empty(), "{k:?}");
            }
        }
    }
}

/// The 555 demos (no microcontroller) on the desktop's session sim.
#[cfg(test)]
mod analog_tests {
    use super::*;
    use crate::supply::SupplyMode;

    fn start555(kind: DemoKind) -> SessionSim {
        SessionSim::start(kind, StartOptions::default()).unwrap_or_else(|e| panic!("{kind:?}: {e}"))
    }

    /// Edge times (s) and directions (`true` rising) of a watched signal.
    type Edges = std::sync::Arc<std::sync::Mutex<Vec<(f64, bool)>>>;

    /// Records the time of every event it receives: code 1 is a rising
    /// crossing, anything else a falling one.
    struct EdgeLog {
        t: SimTime,
        log: Edges,
    }

    impl of_core::Engine for EdgeLog {
        fn name(&self) -> &str {
            "edge log"
        }
        fn local_time(&self) -> SimTime {
            self.t
        }
        fn advance_to(
            &mut self,
            limit: SimTime,
            _: &mut of_core::SyncCtx<'_>,
        ) -> Result<SimTime, of_core::EngineError> {
            self.t = limit;
            Ok(limit)
        }
        fn on_event(&mut self, ev: &of_core::Event, _: &mut of_core::SyncCtx<'_>) -> Result<(), of_core::EngineError> {
            self.log.lock().unwrap().push((ev.at.as_secs_f64(), ev.code == 1));
            Ok(())
        }
        fn is_quiescent(&self) -> bool {
            true
        }
    }

    /// Times every crossing of `signal` through `level` as the solver
    /// locates it (within the analog engine's crossing resolution), not on
    /// the 50 us sample grid.
    fn watch_edges(s: &mut SessionSim, signal: &str, level: f64) -> Edges {
        let log = Edges::default();
        let t = s.s.kernel().now();
        let id = s.s.kernel().add_engine(Box::new(EdgeLog {
            t,
            log: std::sync::Arc::clone(&log),
        }));
        for (edge, code) in [(of_analog::Edge::Rising, 1), (of_analog::Edge::Falling, 2)] {
            s.s.analog().watch(signal, level, edge, id.0, code).unwrap();
        }
        log
    }

    /// Duty (%), frequency (Hz) and count of the whole periods (rising edge
    /// to rising edge) in `edges` from `from` on.
    fn edge_duty_freq(edges: &[(f64, bool)], from: f64) -> Option<(f64, f64, usize)> {
        let e: Vec<(f64, bool)> = edges.iter().copied().filter(|&(t, _)| t >= from).collect();
        let first = e.iter().position(|&(_, r)| r)?;
        let last = e.iter().rposition(|&(_, r)| r)?;
        let span = e[last].0 - e[first].0;
        let (mut high, mut rise, mut rises) = (0.0, None, 0usize);
        for &(t, r) in &e[first..=last] {
            if r {
                rise = Some(t);
                rises += 1;
            } else if let Some(t0) = rise.take() {
                high += t - t0;
            }
        }
        let periods = rises.checked_sub(1).filter(|&n| n > 0)?;
        Some((100.0 * high / span, periods as f64 / span, periods))
    }

    /// The face reads the file's 5 V supply in CV, inside the NE555's
    /// 4.5 to 16 V.
    fn check_supply(s: &SessionSim, kind: DemoKind) {
        let st = s.supply_status();
        println!(
            "{kind:?}: supply {:.4} V, {:.3} mA, {:?}, outside {}",
            st.volts,
            st.amps * 1e3,
            st.mode,
            st.outside_rated
        );
        assert!((st.volts - 5.0).abs() < 0.01, "{kind:?}: {} V", st.volts);
        assert_eq!((st.mode, st.outside_rated), (SupplyMode::Cv, false), "{kind:?}");
    }

    /// Samples on the LED channel after `from` (s).
    fn samples_after(s: &SessionSim, from: f64) -> usize {
        s.led_ma().iter().filter(|(t, _)| *t > from + 1e-9).count()
    }

    /// Blink's loop after a stop off the 50 us grid, as a pot change's 1 us
    /// step makes, and at 60 Hz frame stops, keeps every grid instant (the
    /// old loop jumped to `until` on a missed one: 458 of 599 and 668 of 999).
    #[test]
    fn blink_loop_keeps_the_grid_after_an_off_grid_stop() {
        let mut s = SessionSim::start(DemoKind::Blink, StartOptions::default()).unwrap();
        s.advance(30e-3).unwrap();
        s.advance(30e-3 + 1e-6).unwrap();
        s.advance(60e-3).unwrap();
        let after_nudge = samples_after(&s, 30.001e-3);
        let mut t = 60e-3;
        for _ in 0..3 {
            t += 1.0 / 60.0;
            s.advance(t).unwrap();
        }
        let after_frames = samples_after(&s, 60e-3);
        println!(
            "blink: {after_nudge} samples in 30.001..60 ms (grid 599), {after_frames} in 60..{:.3} ms (grid {})",
            t * 1e3,
            ((t - 60e-3) / 50e-6).floor()
        );
        assert!(after_nudge >= 599, "{after_nudge} samples after the off-grid stop");
        assert!(
            after_frames as f64 >= ((t - 60e-3) / 50e-6).floor(),
            "{after_frames} samples over the frames"
        );
    }

    /// Sim seconds per wall second for each 555 demo (release builds are
    /// the meaningful number; the PWM dimmer is on the 5 us grid).
    #[test]
    fn ne555_realtime_factor_report() {
        const SPAN: f64 = 0.2;
        for kind in [DemoKind::Ne555Astable, DemoKind::Ne555Monostable, DemoKind::Ne555Pwm] {
            let mut s = start555(kind);
            s.advance(0.02).unwrap();
            let t0 = std::time::Instant::now();
            s.advance(0.02 + SPAN).unwrap();
            let wall = t0.elapsed().as_secs_f64().max(1e-9);
            println!(
                "{kind:?} realtime factor: {:.3}x ({SPAN} s sim in {wall:.3} s wall)",
                SPAN / wall
            );
        }
    }

    #[test]
    fn astable_flashes_at_about_6_7_hz() {
        let kind = DemoKind::Ne555Astable;
        assert!(!kind.has_mcu() && !kind.needs_rom() && !kind.has_pot() && !kind.has_button());
        let mut s = start555(kind);
        // The face's 20 ms PWM readout cannot see a 149 ms period, so read
        // the LED current each millisecond and time the rises.
        let (mut rises, mut lit, mut was) = (Vec::new(), 0usize, false);
        for k in 1..=1000 {
            let t = f64::from(k) * 1e-3;
            s.advance(t).unwrap();
            let on = s.current_ma().unwrap() > 0.5;
            if on && !was {
                rises.push(t);
            }
            if t > 0.3 && on {
                lit += 1;
            }
            was = on;
        }
        let periods: Vec<f64> = rises.windows(2).skip(1).map(|w| w[1] - w[0]).collect();
        let period = periods.iter().sum::<f64>() / periods.len() as f64;
        let duty = lit as f64 / 700.0;
        println!(
            "{kind:?}: rises {rises:?}, period {:.1} ms ({:.2} Hz), duty {:.1} %, average {:.3} mA",
            period * 1e3,
            1.0 / period,
            duty * 100.0,
            s.average_ma()
        );
        // 0.693 (R1 + 2 R2) C plus the discharge drop: about 149 ms, 6.7 Hz.
        assert!(periods.len() >= 4, "{rises:?}");
        assert!((period - 0.149).abs() < 0.004, "period {period}");
        assert!((0.48..0.58).contains(&duty), "duty {duty}");
        check_supply(&s, kind);
        assert!(s.uart_bytes().is_empty() && s.uart_baud().is_none());
    }

    #[test]
    fn pwm_dimmer_follows_the_pot() {
        let kind = DemoKind::Ne555Pwm;
        assert!(kind.has_pot() && !kind.has_mcu());
        let mut s = start555(kind);
        let mut t = 0.0;
        // The output's edges at mid-swing, timed by the solver.
        let edges = watch_edges(&mut s, "v(out)", 2.0);
        // The exact duty % and mean LED current (mA) of the of-circuit test.
        // Exact period (ms) too. Edges are timed off the capacitor ramps.
        let wall = std::time::Instant::now();
        for (pos, want, mean, period_ms) in [
            (0.5, 52.99, 5.005, 1.0328),
            (0.9, 89.68, 8.484, 1.0027),
            (0.1, 18.00, 1.695, 1.0458),
            (0.3, 35.44, 3.347, 1.0413),
            (0.7, 70.94, 6.708, 1.0204),
        ] {
            s.set_position(pos).unwrap();
            // Skip start-up and the pot's settling (about 5 periods), then
            // measure over the whole periods left in the 30 ms.
            let from = t + 5e-3;
            t += 30e-3;
            s.advance(t).unwrap();
            let shown = s.pwm_duty_freq();
            let measured = edge_duty_freq(&edges.lock().unwrap(), from);
            println!(
                "{kind:?} at {pos}: edge-timed (duty %, Hz, periods) {measured:?}, face {shown:?}, average {:.3} mA",
                s.average_ma()
            );
            // Whole periods between solver-timed edges: within 0.5 points
            // and 1 % of the exact duty and frequency.
            let (d, f, periods) = measured.expect("whole periods");
            assert!(periods >= 20, "pot {pos}: {periods} periods");
            assert!((d - want).abs() < 0.5, "pot {pos}: duty {d} % vs {want} %");
            let f_exact = 1.0 / period_ms * 1e3;
            assert!((f / f_exact - 1.0).abs() < 0.01, "pot {pos}: {f} Hz vs {f_exact} Hz");
            let avg = s.average_ma();
            // Whole periods on the 50 us grid: at most one sample of the
            // 9.47 mA lit current per 1 ms period (0.47 mA) off.
            assert!((avg - mean).abs() < 0.47, "pot {pos}: {avg} mA vs {mean} mA");
            // The file's multimeter (XMM1, LED current) reads the same way.
            let mm = s.file_meter_readings();
            assert_eq!((mm[0].0.as_str(), mm[0].1.as_str()), ("XMM1", "LED current"));
            println!("  XMM1 {}", mm[0].2);
        }
        println!(
            "{kind:?}: 150 ms of sim on the 50 us grid in {:?} (debug build)",
            wall.elapsed()
        );
        check_supply(&s, kind);
    }

    /// The desktop scope and meters come from each file's `[[instrument]]`
    /// tables: the astable's XSC1 shows the capacitor between VCC/3 and
    /// 2 VCC/3 and the output swinging, XMM1 the LED current.
    #[test]
    fn the_555_instruments_come_from_the_file() {
        let mut s = start555(DemoKind::Ne555Astable);
        s.advance(0.5).unwrap();
        let scopes = s.file_scope_traces();
        assert_eq!(scopes.len(), 1);
        let (reference, chans) = &scopes[0];
        let names: Vec<&str> = chans.iter().map(|(n, _)| n.as_str()).collect();
        assert_eq!(
            (reference.as_str(), names),
            ("XSC1", vec!["Timing capacitor", "Output"])
        );
        let span = |pts: &[(f64, f64)]| {
            pts.iter()
                .filter(|(t, _)| *t > 0.3)
                .fold((f64::MAX, f64::MIN), |(a, b), &(_, v)| (a.min(v), b.max(v)))
        };
        let (cmin, cmax) = span(&chans[0].1);
        let (omin, omax) = span(&chans[1].1);
        println!("XSC1 Timing capacitor {cmin:.3}..{cmax:.3} V, Output {omin:.3}..{omax:.3} V");
        assert!(
            (cmin - 5.0 / 3.0).abs() < 0.1 && (cmax - 10.0 / 3.0).abs() < 0.1,
            "{cmin} {cmax}"
        );
        assert!(omin < 0.3 && omax > 3.0, "{omin} {omax}");
        let mm = s.file_meter_readings();
        println!("XMM1 {mm:?}");
        assert_eq!(mm.len(), 1);
        // A DC meter's 20 ms window on a 6.7 Hz flasher follows the flash:
        // at 0.5 s it sits inside the lit 76 ms that began at 0.491 s.
        let ma: f64 = mm[0].2.trim_end_matches(" mA").parse().unwrap();
        assert!((ma - 9.465).abs() < 0.1, "{ma} mA");
        // The ESP32-C3 demos keep their own instruments.
        let blink = SessionSim::start(DemoKind::Blink, StartOptions::default()).unwrap();
        assert!(blink.file_scope_traces().is_empty() && blink.file_meter_readings().is_empty());
    }

    /// The clock counter: the frequency the firmware prints on UART0 is the
    /// one the file's scope sees at GPIO4 (rising mid-rail crossings over
    /// whole periods), and both sit near the NE555's 1/(ln 2 (R1 + 2 R2) C1).
    #[test]
    fn the_clock_counter_prints_the_frequency_the_scope_sees() {
        let kind = DemoKind::Clock555;
        assert!(kind.has_mcu() && !kind.needs_rom() && kind.file_bench());
        let mut s = SessionSim::start(kind, StartOptions::default()).unwrap();
        assert!((s.supply.volts - 5.0).abs() < 1e-12, "starts on the file's 5 V");
        // The first 100 ms gate closes at about 117 ms (the first edge after
        // the 2 ms settle is at 13.6 ms) and its line takes about 10 ms at
        // 28818 baud; one gate keeps the debug run short.
        s.advance(0.135).unwrap();
        let consoles = s.consoles();
        assert_eq!(consoles.len(), 1);
        let text = &consoles[0].text;
        println!("{}:\n{text}", consoles[0].title);
        assert!(text.contains("ohmforge clock counter"), "{text:?}");
        // Only whole lines: the last one may still be on the wire.
        let printed: Vec<f64> = text
            .split_inclusive('\n')
            .filter(|l| l.ends_with('\n'))
            .filter_map(|l| l.trim_end().strip_prefix("freq: "))
            .map(|r| r.split(' ').next().unwrap().parse().unwrap())
            .collect();
        assert!(!printed.is_empty(), "{text:?}");
        let scopes = s.file_scope_traces();
        let clk = &scopes[0]
            .1
            .iter()
            .find(|(n, _)| n == "Clock at GPIO4")
            .expect("clock channel")
            .1;
        let mut edges = Vec::new();
        for w in clk.windows(2) {
            let ((t0, v0), (t1, v1)) = (w[0], w[1]);
            if t0 > 0.05 && v0 < 1.6 && v1 >= 1.6 {
                edges.push(t0 + (1.6 - v0) / (v1 - v0) * (t1 - t0));
            }
        }
        assert!(edges.len() >= 3, "{} edges", edges.len());
        let measured = (edges.len() - 1) as f64 / (edges[edges.len() - 1] - edges[0]);
        let expected = 1.0 / (std::f64::consts::LN_2 * (1e3 + 2.0 * 6.8e3) * 1e-6);
        println!(
            "printed {printed:?} Hz, scope {measured:.3} Hz over {} periods, 1/(ln2 (R1 + 2 R2) C1) = {expected:.3} Hz",
            edges.len() - 1
        );
        for f in &printed {
            assert!(
                (f - measured).abs() / measured < 2e-3,
                "printed {f} vs scope {measured}"
            );
        }
        assert!(
            (measured - expected).abs() / expected < 0.03,
            "{measured} vs {expected}"
        );
        let logic = s.logic();
        assert_eq!(logic.len(), 1, "one logic analyser (clock and TX)");
    }

    /// Supply warnings may only name parts on each demo's PSU + rail: the
    /// 555 boards (the clock board too, its C3 is behind the LDO) name the
    /// NE555, the other ESP32-C3 boards name U1. Every part named is on the
    /// board.
    #[test]
    fn supply_warnings_name_only_parts_on_the_rail() {
        for &kind in KINDS.iter() {
            let rail = rail_parts(kind);
            let file = parsed(kind).unwrap();
            for p in &rail {
                assert!(
                    file.components
                        .iter()
                        .any(|c| c.reference() == p.reference && c.part_id() == p.part),
                    "{kind:?}: {p:?} is not on the board"
                );
            }
            let ids: Vec<&str> = rail.iter().map(|p| p.part.as_str()).collect();
            if kind.has_mcu() && kind != DemoKind::Clock555 {
                assert_eq!(ids, ["espressif.esp32-c3"], "{kind:?}");
            } else {
                assert_eq!(ids, ["ti.ne555"], "{kind:?}");
            }
        }
    }

    /// Out of range, the warning names the rail's parts at their ratings: a
    /// 555 board at 3.3 V names the NE555 (no ESP32-C3), the ESP32-C3 blink
    /// board at 4 V names U1; each board on its own file supply is quiet.
    #[test]
    fn out_of_range_supplies_warn_with_the_parts_present() {
        use crate::supply::rating_warning;
        let warning = |kind: DemoKind, set: SupplySettings| {
            let mut s = SessionSim::start(
                kind,
                StartOptions {
                    supply: Some(set),
                    ..StartOptions::default()
                },
            )
            .unwrap();
            s.advance(2e-3).unwrap();
            rating_warning(&s.supply_status(), &set, &rail_parts(kind))
        };
        for kind in [
            DemoKind::Ne555Astable,
            DemoKind::Ne555Monostable,
            DemoKind::Ne555Pwm,
            DemoKind::Clock555,
        ] {
            let file = file_supply(kind);
            assert_eq!(warning(kind, file), None, "{kind:?} on its file supply");
            let w = warning(kind, SupplySettings { volts: 3.3, ..file }).expect("3.3 V is below the NE555's 4.5 V");
            assert!(w.contains("ti.ne555 4.5\u{2013}16 V"), "{kind:?}: {w}");
            assert!(!w.to_lowercase().contains("esp32"), "{kind:?}: {w}");
        }
        let blink = file_supply(DemoKind::Blink);
        assert_eq!(warning(DemoKind::Blink, blink), None);
        let w = warning(DemoKind::Blink, SupplySettings { volts: 4.0, ..blink }).expect("4 V is above 3.6 V");
        assert!(w.contains("U1 espressif.esp32-c3 3\u{2013}3.6 V"), "{w}");
    }

    /// Out-of-range bench settings on the four 555 boards simulate (and
    /// warn), promptly: never a panic, a hang or a solver error. The cold
    /// starts that used to stop in "time step too small" (the clock at 5 V,
    /// 10 milliohms and 60 mA and at 2 V, the PWM board at 30 V: the NE555
    /// latch chattering with every input inside its comparators' 1 mV band
    /// near VCC = 0) run since the model's undervoltage lockout holds the
    /// latch reset there. Any error still goes to the status strip with the
    /// panels drawn. The case from the field, the clock changed to 3.3 V,
    /// 10 milliohms and 60 mA while running, runs.
    #[test]
    fn out_of_range_supplies_simulate_or_report_on_every_555_board() {
        let combos = [
            (3.3, 0.01, 0.06),
            (3.3, 0.1, 0.5),
            (5.0, 0.01, 0.06),
            (2.0, 0.1, 0.5),
            (0.0, 0.1, 0.5),
            (5.0, 0.1, 0.005),
            (30.0, 0.1, 0.5),
        ];
        let known: Vec<(DemoKind, f64, f64, f64)> = Vec::new();
        let mut failed = Vec::new();
        for kind in [
            DemoKind::Ne555Astable,
            DemoKind::Ne555Monostable,
            DemoKind::Ne555Pwm,
            DemoKind::Clock555,
        ] {
            for (v, r, i) in combos {
                let supply = SupplySettings {
                    volts: v,
                    series_r: r,
                    current_limit: i,
                    ..file_supply(kind)
                };
                let t0 = std::time::Instant::now();
                let res = SessionSim::start(
                    kind,
                    StartOptions {
                        supply: Some(supply),
                        ..StartOptions::default()
                    },
                )
                .and_then(|mut s| s.advance(5e-3).map(|()| s));
                let wall = t0.elapsed().as_secs_f64();
                assert!(wall < 60.0, "{kind:?} {v} V {r} ohm {i} A took {wall:.1} s");
                match res {
                    Ok(s) => assert!(s.time() >= 5e-3 - 1e-9, "{kind:?}"),
                    Err(e) => {
                        println!("{kind:?} {v} V {r} ohm {i} A: {e}");
                        assert!(e.contains("time step too small"), "{kind:?} {v} V {r} ohm {i} A: {e}");
                        failed.push((kind, v, r, i));
                    }
                }
            }
        }
        println!("cold starts that end in a reported error: {failed:?}");
        assert_eq!(failed, known, "the reported errors changed");
        let mut s = SessionSim::start(DemoKind::Clock555, StartOptions::default()).unwrap();
        s.advance(5e-3).unwrap();
        s.set_supply(SupplySettings {
            volts: 3.3,
            series_r: 0.01,
            current_limit: 0.06,
            ..file_supply(DemoKind::Clock555)
        })
        .unwrap();
        s.advance(15e-3).unwrap();
    }

    #[test]
    fn monostable_lights_for_1_1_rc_on_a_press() {
        let kind = DemoKind::Ne555Monostable;
        assert!(kind.has_button() && !kind.has_mcu());
        let mut s = start555(kind);
        s.advance(20e-3).unwrap();
        let idle = s.current_ma().unwrap();
        s.set_closed(true).unwrap();
        s.advance(50e-3).unwrap();
        s.set_closed(false).unwrap();
        s.advance(100e-3).unwrap();
        let lit = s.current_ma().unwrap();
        s.advance(160e-3).unwrap();
        let after = s.current_ma().unwrap();
        println!("{kind:?}: LED {idle:.3} mA idle, {lit:.3} mA at 100 ms, {after:.3} mA at 160 ms");
        // RC ln 3 = 109.9 ms from the press at 20 ms: lit at 100 ms, off at 160.
        assert!(idle < 0.01, "idle {idle} mA");
        assert!(lit > 5.0, "lit {lit} mA");
        assert!(after < 0.01, "after {after} mA");
        check_supply(&s, kind);
    }
}
