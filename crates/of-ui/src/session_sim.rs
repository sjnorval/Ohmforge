//! The desktop demos on [`of_engine::Session`], built from their circuit
//! files (engine E4c; `--circuit-files`).
//!
//! [`SessionSim`] runs a built `Session` with the hand-wired demos' sampling
//! loops (scope grid, LEDC events, the button's grid) and offers the same
//! methods the live worker calls on `Blink`, `PwmSim` and `ButtonSim`. The
//! trace parity tests (`parity`) run the same type, so the desktop's file path
//! is the code the goldens check. [`start`] builds a demo from the embedded
//! `circuits/demos` file with the embedded parts and firmware; the ROM-booted
//! chips come from the same pre-boot caches the hand-wired demos use.
//!
//! The shipped files keep the ESP32-C3's supply current (`quiescent_a`, from
//! datasheet v2.5 Table 5-8), as the hand-wired boards do, so both paths read
//! the same.

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

/// The pre-booted chip of a ROM demo (the hand-wired demos' caches), or
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
        // A missing ROM shows the desktop's own message, as on the hand-wired path.
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

/// A demo running on [`Session`] with the hand-wired sampling loop.
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

    /// Display-average LED current (mA), as the hand-wired demo of this kind.
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

    /// The running session (parity steps set raw host params).
    pub(crate) fn session(&mut self) -> &mut Session {
        &mut self.s
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::adc_pwm::PwmSim;
    use crate::blink::Blink;
    use crate::button::ButtonSim;
    use crate::parity::skip_without_rom;

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

    fn run_hand_wired(kind: DemoKind, idle_draw: bool) -> View {
        let supply = SupplySettings::default();
        match kind {
            DemoKind::Blink => {
                let idle = if idle_draw {
                    crate::pins::IDLE_DRAW_RUNNING_A
                } else {
                    0.0
                };
                let mut s = Blink::with_supply(supply, idle).unwrap();
                for t in [1e-3, 2e-3, 3e-3] {
                    s.advance(t).unwrap();
                }
                View {
                    time: s.time(),
                    instant: Some(s.current_ma().unwrap()),
                    average: s.average_ma(),
                    trace: s.trace(),
                    duty: s.pwm_duty_freq(),
                    uart: text(s.uart_bytes()),
                    supply: s.supply_status(),
                    readout: s.circuit_readout(),
                    wiper: None,
                    adc: None,
                    button: None,
                }
            }
            DemoKind::Dimmer | DemoKind::Fade => {
                let pk = if kind == DemoKind::Dimmer {
                    PwmKind::Dimmer
                } else {
                    PwmKind::Fade
                };
                let idle = if idle_draw {
                    crate::pins::IDLE_DRAW_RUNNING_A
                } else {
                    0.0
                };
                let mut s = PwmSim::with_supply(pk, 0.5, supply, idle).unwrap();
                s.advance(1e-3).unwrap();
                if kind == DemoKind::Dimmer {
                    s.set_position(0.8).unwrap();
                }
                s.advance(2e-3).unwrap();
                s.advance(3e-3).unwrap();
                View {
                    time: s.time(),
                    instant: None,
                    average: s.average_ma(),
                    trace: s.trace(),
                    duty: s.pwm_duty_freq(),
                    uart: text(s.uart_bytes()),
                    supply: s.supply_status(),
                    readout: s.circuit_readout(),
                    wiper: (kind == DemoKind::Dimmer).then(|| s.wiper_volts().unwrap()),
                    adc: if kind == DemoKind::Dimmer {
                        s.last_adc_uart()
                    } else {
                        None
                    },
                    button: None,
                }
            }
            DemoKind::ButtonPoll | DemoKind::ButtonIrq => {
                let bk = if kind == DemoKind::ButtonPoll {
                    ButtonKind::Poll
                } else {
                    ButtonKind::Irq
                };
                let idle = if idle_draw { bk.idle_draw_a() } else { 0.0 };
                let mut s = ButtonSim::with_supply(bk, supply, idle).unwrap();
                s.advance(1e-3).unwrap();
                s.set_closed(true).unwrap();
                s.advance(2e-3).unwrap();
                s.set_closed(false).unwrap();
                s.advance(3e-3).unwrap();
                View {
                    time: s.time(),
                    instant: Some(s.current_ma().unwrap()),
                    average: s.average_ma(),
                    trace: s.trace(),
                    duty: (None, None),
                    uart: text(s.uart_bytes()),
                    supply: s.supply_status(),
                    readout: s.circuit_readout(),
                    wiper: None,
                    adc: None,
                    button: Some(s.button_volts().unwrap()),
                }
            }
        }
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

    /// The file path and the hand-wired path, both with or both without the
    /// ESP32-C3's supply current, show the same LED current, scope trace, PWM
    /// readout, UART text, PSU face and meter readout (node voltages, branch
    /// names and currents), within RFC 0003's `max(1 nA, 1 ppm)` and with
    /// no per-demo exceptions.
    fn file_reads_like_the_hand_wired_demo(idle_draw: bool) {
        for kind in KINDS {
            let Some(file) = run_file(kind, idle_draw) else {
                continue;
            };
            let hand = run_hand_wired(kind, idle_draw);
            let k = format!("{kind:?} (idle draw {idle_draw})");
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

    /// As shipped: both paths draw the ESP32-C3's datasheet supply current.
    #[test]
    fn every_demo_from_its_file_reads_like_the_hand_wired_demo() {
        file_reads_like_the_hand_wired_demo(true);
    }

    /// As in the parity runs: the supply current off on both paths.
    #[test]
    fn every_demo_from_its_file_reads_like_the_hand_wired_demo_without_idle_draw() {
        file_reads_like_the_hand_wired_demo(false);
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
}
