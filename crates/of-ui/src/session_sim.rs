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
use of_soc_esp32c3::{Esp32c3Engine, UartByte};

use crate::adc_pwm::{self, PwmKind};
use crate::button::{self, ButtonKind};
use crate::kind::DemoKind;
use crate::meters::{self, CircuitReadout};
use crate::supply::{RATED_MAX_V, RATED_MIN_V, SupplyMode, SupplySettings, SupplyStatus};
use crate::trace::{self, SampleClock, TraceBuf};

const R_SERIES: f64 = 330.0;
/// Display-average window of the PWM demos (as `PwmSim`).
const PWM_AVG_WINDOW: f64 = 2e-3;
/// Display-average window of the button demos (as `ButtonSim`).
const BUTTON_AVG_WINDOW: f64 = 2e-3;
const APB_HZ: u64 = 80_000_000;

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
    samples: TraceBuf,
    psu_samples: TraceBuf,
    clock: SampleClock,
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
        let mut d = Self {
            kind,
            s,
            style,
            samples: TraceBuf::default_scope(),
            psu_samples: TraceBuf::default_scope(),
            clock: SampleClock::new(0.0, trace::SAMPLE_STEP),
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
            d.clock.take(d.time());
            d.next_sample = SimTime::from_secs_f64(d.clock.next().max(0.0));
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

    fn instant_ma(&mut self) -> Result<f64, String> {
        let gpio = self.s.value("v(gpio8)").ok_or("no GPIO8 voltage in the solution")?;
        let anode = self.s.value("v(led_a)").ok_or("no LED anode voltage in the solution")?;
        Ok((gpio - anode) * 1e3 / R_SERIES)
    }

    fn record_sample(&mut self) -> Result<(), String> {
        let t = self.time();
        let ma = self.instant_ma()?;
        self.samples.push(t, ma);
        self.cpu().recycle_consumed_edges();
        self.s.commit().map_err(|e| e.to_string())?;
        self.supply_status = self.read_supply();
        self.psu_samples.push(t, self.supply_status.amps * 1e3);
        Ok(())
    }

    /// The PSU's state after its last commit, as `supply::regulate` reports it.
    fn read_supply(&mut self) -> SupplyStatus {
        let mut volts = 0.0;
        let mut amps = 0.0;
        let mut mode = SupplyMode::Off;
        for (k, v) in self.s.readings("PSU") {
            match k.as_str() {
                "volts" => volts = v,
                "regulated_amps" => amps = v,
                "mode" => {
                    mode = match v as i64 {
                        1 => SupplyMode::Cv,
                        2 => SupplyMode::Cc,
                        _ => SupplyMode::Off,
                    }
                }
                _ => {}
            }
        }
        let rated = RATED_MIN_V..=RATED_MAX_V;
        let outside_rated = if self.supply.on {
            !rated.contains(&self.supply.volts) || !rated.contains(&volts)
        } else {
            !rated.contains(&volts) && volts > 0.05
        };
        SupplyStatus {
            volts,
            amps,
            amps_avg: None,
            mode,
            outside_rated,
        }
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
            let grid = SimTime::from_secs_f64(self.clock.next().max(0.0));
            let next = grid.min(until);
            if next > self.s.now() {
                self.run_to(next)?;
            } else if self.s.now() < until {
                self.run_to(until)?;
            }
            if self.clock.take(self.time()) {
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
            let grid = SimTime::from_secs_f64(self.clock.next().max(0.0));
            let pwm = self
                .cpu()
                .soc
                .bus
                .ledc
                .next_event()
                .map(|t| SimTime(t.0.saturating_sub(origin.0)));
            if grid <= now {
                if self.clock.take(self.time()) {
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
            let hit_grid = self.clock.take(self.time());
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
                if self.clock.take(self.time()) {
                    self.record_sample()?;
                }
                self.next_sample = SimTime::from_secs_f64(self.clock.next().max(0.0));
                if self.next_sample <= self.s.now() {
                    self.next_sample = self.s.now().saturating_add(SimTime::from_ns(1));
                }
            }
        }
        Ok(())
    }

    pub(crate) fn set_scope_window(&mut self, visible: f64) {
        self.samples.set_visible_window(visible);
        self.psu_samples.set_visible_window(visible);
        self.clock.set_step(self.samples.sample_step(), self.time());
        if self.style == Loop::Button {
            self.next_sample = SimTime::from_secs_f64(self.clock.next().max(0.0));
        }
    }

    /// Instantaneous LED current (mA).
    pub(crate) fn current_ma(&mut self) -> Result<f64, String> {
        self.instant_ma()
    }

    /// Display-average LED current (mA), averaged as for this kind.
    pub(crate) fn average_ma(&self) -> f64 {
        let pts = self.samples.to_vec();
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
        if self.style != Loop::Pwm {
            return self.samples.to_vec();
        }
        let pts = self.samples.to_vec();
        pts.iter()
            .map(|&(t, _)| {
                (
                    t,
                    trace::average_periods(
                        &pts,
                        &pts,
                        t,
                        trace::PWM_EDGE_HIGH,
                        trace::PWM_AVG_PERIODS,
                        PWM_AVG_WINDOW,
                    ),
                )
            })
            .collect()
    }

    pub(crate) fn pwm_duty_freq(&self) -> (Option<f64>, Option<f64>) {
        trace::pwm_duty_freq(&self.samples.to_vec(), self.time(), 0.5)
    }

    pub(crate) fn supply_status(&self) -> SupplyStatus {
        let led = self.samples.to_vec();
        let psu = self.psu_samples.to_vec();
        let window = match self.style {
            Loop::Pwm => PWM_AVG_WINDOW,
            Loop::Blink | Loop::Button => trace::LED_AVG_WINDOW,
        };
        let mut status = self.supply_status;
        status.amps_avg = Some(
            trace::average_periods(
                &psu,
                &led,
                self.time(),
                trace::PWM_EDGE_HIGH,
                trace::PWM_AVG_PERIODS,
                window,
            ) * 1e-3,
        );
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

    /// Node voltages and branch currents for the meters, with the same names
    /// as the hand-wired demos (ground is `0`, branches `R1`/`R2`, `D1`,
    /// `RV1 (1–W)`, `RV1 (W–3)`, `SW1`, `PSU`).
    pub(crate) fn circuit_readout(&mut self) -> CircuitReadout {
        let mut out = CircuitReadout::new();
        let mut nodes = vec!["gpio8", "led_a", "vdd"];
        match self.kind {
            DemoKind::Dimmer => nodes.push("gpio2"),
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => nodes.push("gpio9"),
            DemoKind::Blink | DemoKind::Fade => {}
        }
        meters::take_nodes(self.s.analog(), &nodes, &mut out);
        meters::add_led_series(&mut out, self.resistor_ref(), "D1", "gpio8", "led_a", R_SERIES);
        match self.kind {
            DemoKind::Dimmer => {
                if let Some(i) = self.s.terminal_currents("RV1") {
                    // Pins 1, W, 3: current into pin 1 flows 1 → W; out of pin 3, W → 3.
                    out.push_branch("RV1 (1–W)", "0", "gpio2", i[0]);
                    out.push_branch("RV1 (W–3)", "gpio2", "vdd", -i[2]);
                }
            }
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => {
                if let Some(i) = self.s.terminal_currents("SW1") {
                    out.push_branch("SW1", "gpio9", "0", i[0]);
                }
            }
            DemoKind::Blink | DemoKind::Fade => {}
        }
        meters::add_psu(&mut out, self.supply_status().display_amps());
        out
    }
}

#[cfg(test)]
impl SessionSim {
    /// Raw outputs for the parity goldens.
    pub(crate) fn parity_raw(&mut self) -> crate::parity::Raw {
        let origin = self.s.origin();
        let led = self.samples.to_vec();
        let psu = self.psu_samples.to_vec();
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
        let samples = self.samples.len();
        crate::stats::PerfSnapshot::from_session(&mut self.s, time, samples)
    }

    /// Capacity of the scope ring.
    #[cfg(all(feature = "gui", test))]
    pub(crate) fn trace_capacity(&self) -> usize {
        self.samples.capacity()
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
        readout: CircuitReadout,
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
            readout: s.circuit_readout(),
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
            let (fr, hr) = (&file.readout, &hand.readout);
            assert_eq!(
                fr.nodes.keys().collect::<Vec<_>>(),
                hr.nodes.keys().collect::<Vec<_>>(),
                "{k} nodes"
            );
            for (name, v) in &hr.nodes {
                assert!(
                    close(fr.nodes[name], *v, VOLTS),
                    "{k} node {name}: {} vs {v}",
                    fr.nodes[name]
                );
            }
            let names = |r: &CircuitReadout| {
                r.branches
                    .iter()
                    .map(|b| (b.name.clone(), b.a.clone(), b.b.clone()))
                    .collect::<Vec<_>>()
            };
            assert_eq!(names(fr), names(hr), "{k} branches");
            for (f, h) in fr.branches.iter().zip(&hr.branches) {
                assert!(
                    close(f.amps, h.amps, AMPS),
                    "{k} branch {}: {} vs {}",
                    h.name,
                    f.amps,
                    h.amps
                );
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
        let vdd = s.circuit_readout().volts("vdd").unwrap();
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
        let r_series = 330.0;
        let mut s = start(DemoKind::Blink, StartOptions::default()).expect("blink needs no ROM");
        s.advance(125e-6).unwrap();
        let r = s.circuit_readout();
        let i_r = r.unique_current("gpio8", "led_a").unwrap();
        let i_d = r.unique_current("led_a", "0").unwrap();
        assert_eq!(i_r.0, "R1");
        assert_eq!(i_d.0, "D1");
        let analytic = (r.volts("gpio8").unwrap() - r.volts("led_a").unwrap()) / r_series;
        assert!((i_r.1 - analytic).abs() < 1e-9, "{} vs {analytic}", i_r.1);
        assert!((i_d.1 - analytic).abs() < 1e-9, "{} vs {analytic}", i_d.1);
        assert!((3.0..4.5).contains(&(analytic * 1e3)), "{} mA", analytic * 1e3);
        // Same current, opposite sign, if the leads are swapped.
        assert!((r.unique_current("led_a", "gpio8").unwrap().1 + analytic).abs() < 1e-9);
    }

    #[test]
    fn dimmer_pot_divider_voltage_matches_the_wiper() {
        let Some(mut s) = dimmer(0.5) else { return };
        let r = s.circuit_readout();
        let vdd = r.volts("vdd").unwrap();
        let wiper = r.voltage_between("gpio2", "0").unwrap();
        assert!((wiper - 0.5 * vdd).abs() < 0.05, "wiper {wiper} V, VDD {vdd} V");
        // The two pot legs are unique branches and their currents match.
        let i_a = r.unique_current("0", "gpio2").expect("pin 1 to wiper");
        let i_b = r.unique_current("vdd", "gpio2").expect("pin 3 to wiper");
        assert!(
            (i_a.1 + i_b.1).abs() < 5e-6,
            "unloaded wiper: {} A vs {} A",
            i_a.1,
            i_b.1
        );
        s.set_supply(SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        })
        .unwrap();
        let r = s.circuit_readout();
        let wiper = r.voltage_between("gpio2", "0").unwrap();
        assert!(
            (wiper - 0.5 * r.volts("vdd").unwrap()).abs() < 0.08,
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
            let v = s.circuit_readout().volts("vdd").unwrap();
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
