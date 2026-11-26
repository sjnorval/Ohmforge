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
    }
}

/// The embedded file of `kind`.
pub(crate) fn demo_file(kind: DemoKind) -> &'static DemoFile {
    demos::demo(file_stem(kind)).expect("every desktop demo has an embedded circuit file")
}

const KINDS: [DemoKind; 5] = [
    DemoKind::Blink,
    DemoKind::Dimmer,
    DemoKind::Fade,
    DemoKind::ButtonPoll,
    DemoKind::ButtonIrq,
];

/// The parsed embedded file of `kind` (parsed once).
pub(crate) fn parsed(kind: DemoKind) -> Result<&'static CircuitFile, String> {
    static FILES: [OnceLock<Result<CircuitFile, String>>; 5] = [const { OnceLock::new() }; 5];
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
                lib.add(p.clone(), None).map_err(|e| e.to_string())?;
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
        DemoKind::Blink => return Ok(None),
        DemoKind::Dimmer => adc_pwm::booted(PwmKind::Dimmer)?,
        DemoKind::Fade => adc_pwm::booted(PwmKind::Fade)?,
        DemoKind::ButtonPoll => button::booted(ButtonKind::Poll)?,
        DemoKind::ButtonIrq => button::booted(ButtonKind::Irq)?,
    };
    Ok(Some(Firmware::Booted(Box::new(soc))))
}

/// Overrides applied to the file before it is built.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct StartOptions {
    /// Bench supply settings (`PSU` params).
    pub supply: SupplySettings,
    /// Pot wiper position (`RV1 position`, dimmer only).
    pub position: f64,
    /// Keep the file's ESP32-C3 supply current (`false` forces
    /// `quiescent_a = 0`, as the parity runs do on both paths).
    pub idle_draw: bool,
}

impl Default for StartOptions {
    fn default() -> Self {
        Self {
            supply: SupplySettings::default(),
            position: 0.5,
            idle_draw: true,
        }
    }
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
    let s = opts.supply.sanitized();
    set_param(&mut file, "PSU", "volts", ParamValue::Number(s.volts));
    set_param(&mut file, "PSU", "series_r", ParamValue::Number(s.series_r));
    set_param(&mut file, "PSU", "current_limit", ParamValue::Number(s.current_limit));
    set_param(&mut file, "PSU", "on", ParamValue::Bool(s.on));
    if kind == DemoKind::Dimmer {
        set_param(
            &mut file,
            "RV1",
            "position",
            ParamValue::Number(opts.position.clamp(0.0, 1.0)),
        );
    }
    if !opts.idle_draw {
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
}

impl Loop {
    pub(crate) fn of(kind: DemoKind) -> Self {
        match kind {
            DemoKind::Blink => Self::Blink,
            DemoKind::Dimmer | DemoKind::Fade => Self::Pwm,
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => Self::Button,
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
        let s = build_session(kind, opts)?;
        let mut sim = Self::run(kind, s)?;
        sim.position = opts.position.clamp(0.0, 1.0);
        sim.supply = opts.supply.sanitized();
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
        };
        if edge_log {
            d.cpu().start_edge_log();
        }
        d.record_sample()?;
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
        let stop = self.s.run_until(t).map_err(|e| e.to_string())?;
        reached(stop, self.time())
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

    /// The LED probe's samples in mA, as the scope, averages and goldens use them.
    fn led_ma(&self) -> Vec<(f64, f64)> {
        self.scope.samples(&self.meas)
    }

    fn record_sample(&mut self) -> Result<(), String> {
        self.meas.record(&mut self.s).map_err(|e| e.to_string())?;
        self.meters.record(&mut self.s).map_err(|e| e.to_string())?;
        self.cpu().recycle_consumed_edges();
        self.s.commit().map_err(|e| e.to_string())?;
        self.settled.record(&mut self.s).map_err(|e| e.to_string())?;
        self.supply_status = self.psu.status(&mut self.s, self.supply);
        Ok(())
    }

    pub(crate) fn advance(&mut self, seconds: f64) -> Result<(), String> {
        match self.style {
            Loop::Blink => self.advance_blink(seconds),
            Loop::Pwm => self.advance_pwm(seconds),
            Loop::Button => self.advance_button(seconds),
        }
    }

    fn advance_blink(&mut self, seconds: f64) -> Result<(), String> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        while self.s.now() < until {
            let grid = SimTime::from_secs_f64(self.meas.next_due().max(0.0));
            let next = grid.min(until);
            if next > self.s.now() {
                self.run_to(next)?;
            } else if self.s.now() < until {
                self.run_to(until)?;
            }
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
        }
    }

    /// The scope trace: raw samples, or the PWM demos' moving average.
    pub(crate) fn trace(&self) -> Vec<(f64, f64)> {
        self.scope.trace(&self.meas)
    }

    pub(crate) fn pwm_duty_freq(&self) -> (Option<f64>, Option<f64>) {
        trace::pwm_duty_freq(&self.led_ma(), self.time(), 0.5)
    }

    pub(crate) fn supply_status(&self) -> SupplyStatus {
        let led = self.led_ma();
        let window = match self.style {
            Loop::Pwm => PWM_AVG_WINDOW,
            Loop::Blink | Loop::Button => trace::LED_AVG_WINDOW,
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
        self.cpu().soc.uart_bytes()
    }

    pub(crate) fn uart_baud(&mut self) -> Option<f64> {
        self.cpu().soc.bus.uart0.baud(APB_HZ)
    }

    pub(crate) fn push_uart_rx(&mut self, bytes: &[u8]) {
        if !bytes.is_empty() {
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
                Loop::Blink | Loop::Button => trace::LED_AVG_WINDOW,
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
            supply: SupplySettings {
                current_limit: 0.5e-3,
                ..SupplySettings::default()
            },
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
            DemoKind::Blink | DemoKind::Fade => {}
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
        (DemoKind::Blink, "gpio8", "led_a", "", "-0.000 V", -3.0601937488523236e-10),
        (DemoKind::Blink, "led_a", "gpio8", "", "0.000 V", 3.0601937488523236e-10),
        (DemoKind::Blink, "gpio8", "vdd", "", "-3.298 V", -3.297701601786737e0),
        (DemoKind::Blink, "vdd", "gpio8", "", "3.298 V", 3.297701601786737e0),
        (DemoKind::Blink, "gpio8", "0", "", "0.000 V", 1.267509912249869e-10),
        (DemoKind::Blink, "0", "gpio8", "", "-0.000 V", -1.267509912249869e-10),
        (DemoKind::Blink, "led_a", "vdd", "", "-3.298 V", -3.2977016014807177e0),
        (DemoKind::Blink, "vdd", "led_a", "", "3.298 V", 3.2977016014807177e0),
        (DemoKind::Blink, "led_a", "0", "", "0.000 V", 4.3277036611021924e-10),
        (DemoKind::Blink, "0", "led_a", "", "-0.000 V", -4.3277036611021924e-10),
        (DemoKind::Blink, "vdd", "0", "", "3.298 V", 3.297701601913488e0),
        (DemoKind::Blink, "0", "vdd", "", "-3.298 V", -3.297701601913488e0),
        (DemoKind::Blink, "gpio8", "led_a", "R1", "-0.00 mA", -9.273314390461588e-13),
        (DemoKind::Blink, "led_a", "gpio8", "R1", "0.00 mA", 9.273314390461588e-13),
        (DemoKind::Blink, "led_a", "0", "D1", "-0.00 mA", -9.273314390461588e-13),
        (DemoKind::Blink, "0", "led_a", "D1", "0.00 mA", 9.273314390461588e-13),
        (DemoKind::Blink, "vdd", "0", "PSU", "-22.98 mA", -2.298398086511888e-2),
        (DemoKind::Blink, "0", "vdd", "PSU", "22.98 mA", 2.298398086511888e-2),
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
        (DemoKind::Blink, None, 1.2e-2, 3.297701601913488e0, 2.298398086511888e-2, 2.298398086511888e-2, SupplyMode::Cv, false, 3.291305306999763e-12),
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
        assert_eq!(
            get("d(U1.GPIO8)"),
            Err(ProbeError::NotAnalog("d(U1.GPIO8)".parse().unwrap()))
        );
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
            supply: SupplySettings {
                current_limit: 0.5e-3,
                ..SupplySettings::default()
            },
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
