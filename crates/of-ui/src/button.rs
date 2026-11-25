//! Push-button demos: esp-hal polls or interrupts on GPIO9, LED on GPIO8.

mod schematic;

use std::collections::VecDeque;
use std::error::Error;
use std::sync::OnceLock;

use of_analog::{AnalogEngine, Edge, EngineConfig, Options};
use of_components::analog::{Board, BuiltBoard, Library, Placement, build};
use of_components::bounce::{BounceSettings, bounce_script};
use of_core::{EngineId, Kernel, SimTime, Stop};
use of_parts::parse_part;
use of_soc_esp32c3::{
    Esp32c3, Esp32c3Engine, PinLink, UartByte, VIH_FRACTION, VIL_FRACTION, initial_input_level, input_code,
};

use crate::meters::{self, CircuitReadout};
use crate::pins::{add_pin_drivers, hook_pin};
use crate::rom;
use crate::supply::{self, SupplyMode, SupplySettings, SupplyStatus};
use crate::trace::{self, SampleClock, TraceBuf};

pub(super) use schematic::show as show_schematic;

const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/partitions.bin");
const POLL: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/button_poll.bin");
const IRQ: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-button/button_irq.bin");
const RESISTOR: &str = include_str!("../../../parts/passives/resistor/part.toml");
const LED: &str = include_str!("../../../parts/leds/red-0603/part.toml");
const SWITCH: &str = include_str!("../../../parts/switches/pushbutton-6mm/part.toml");

const R_SERIES: f64 = 330.0;
pub(super) const GPIO_LED: u8 = 8;
pub(super) const GPIO_BTN: u8 = 9;
pub(super) const RESISTOR_REF: &str = "R2";
pub(super) const LED_REF: &str = "D1";
pub(super) const SWITCH_REF: &str = "SW1";
const GPIO_NODE: &str = "v(gpio8)";
const ANODE_NODE: &str = "v(led_a)";
const BTN_NODE: &str = "v(gpio9)";

const POLL_BANNER: &str = "ohmforge esp-hal button (polling)\n";
const IRQ_BANNER: &str = "ohmforge esp-hal button (interrupt)\n";
const AVG_WINDOW: f64 = 2e-3;

static POLL_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();
static IRQ_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();

/// Which committed esp-hal button app to boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ButtonKind {
    Poll,
    Irq,
}

impl ButtonKind {
    fn banner(self) -> &'static str {
        match self {
            Self::Poll => POLL_BANNER,
            Self::Irq => IRQ_BANNER,
        }
    }

    fn app(self) -> &'static [u8] {
        match self {
            Self::Poll => POLL,
            Self::Irq => IRQ,
        }
    }

    fn cache(self) -> &'static OnceLock<Result<Esp32c3, String>> {
        match self {
            Self::Poll => &POLL_SOC,
            Self::Irq => &IRQ_SOC,
        }
    }
}

/// Cached boot result, if the ROM has already been tried for this app.
#[allow(dead_code)]
pub(super) fn cached(kind: ButtonKind) -> Option<&'static Result<Esp32c3, String>> {
    kind.cache().get()
}

/// Boot the app through the fetched ROM (cached after the first success or error).
pub(super) fn booted(kind: ButtonKind) -> Result<Esp32c3, String> {
    kind.cache()
        .get_or_init(|| rom::boot_app(kind.app(), PARTITIONS, kind.banner()))
        .clone()
}

pub(crate) struct ButtonSim {
    #[allow(dead_code)]
    kind: ButtonKind,
    kernel: Kernel,
    cpu: EngineId,
    analog: EngineId,
    board: BuiltBoard,
    /// Intended (held) state from the UI, after bounce has been requested.
    closed: bool,
    /// Actual SW1 contact, which may still be bouncing.
    contact: bool,
    bounce: BounceSettings,
    pending_bounce: VecDeque<(SimTime, bool)>,
    next_sample: SimTime,
    origin: SimTime,
    samples: TraceBuf,
    psu_samples: TraceBuf,
    sample_clock: SampleClock,
    supply: SupplySettings,
    supply_mode: SupplyMode,
    supply_status: SupplyStatus,
}

impl ButtonSim {
    #[allow(dead_code)]
    pub(crate) fn new(kind: ButtonKind) -> Result<Self, Box<dyn Error>> {
        Self::with_supply(kind, SupplySettings::default())
    }

    pub(super) fn with_supply(kind: ButtonKind, supply: SupplySettings) -> Result<Self, Box<dyn Error>> {
        let soc = booted(kind)?;
        Self::from_soc(kind, soc, supply)
    }

    fn from_soc(kind: ButtonKind, soc: Esp32c3, supply: SupplySettings) -> Result<Self, Box<dyn Error>> {
        let supply = supply.sanitized();
        let mut lib = Library::new();
        lib.add(parse_part(RESISTOR)?, None)?;
        lib.add(parse_part(LED)?, None)?;
        lib.add(parse_part(SWITCH)?, None)?;
        let mut board = build(
            &lib,
            &Board {
                placements: vec![
                    Placement::new(SWITCH_REF, "generic.pushbutton", &[("1", "gpio9"), ("2", "0")]),
                    Placement::new(RESISTOR_REF, "generic.resistor", &[("1", "gpio8"), ("2", "led_a")])
                        .with("resistance", R_SERIES),
                    Placement::new(LED_REF, "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
                ],
                ..Board::default()
            },
        )?;
        let vdd = board.circuit.node("vdd");
        supply::add_supply(&mut board.circuit, vdd, &supply)?;
        for pin in [GPIO_BTN, GPIO_LED] {
            let node = board.circuit.node(&format!("gpio{pin}"));
            add_pin_drivers(&mut board.circuit, pin, node, vdd, soc.pin(usize::from(pin)))?;
        }

        let circuit = std::mem::replace(&mut board.circuit, of_analog::Circuit::new());
        let mut analog = AnalogEngine::new(
            "analog",
            circuit,
            Options::default(),
            EngineConfig {
                max_step: 5e-6,
                record: false,
                ..EngineConfig::default()
            },
        )?;
        hook_pin(&mut analog, GPIO_BTN);
        hook_pin(&mut analog, GPIO_LED);
        let vdd_w = supply.volts.max(0.1);
        analog.watch_relative(
            BTN_NODE,
            "v(vdd)",
            VIH_FRACTION,
            Edge::Rising,
            0,
            input_code(GPIO_BTN, true),
        )?;
        analog.watch_relative(
            BTN_NODE,
            "v(vdd)",
            VIL_FRACTION,
            Edge::Falling,
            0,
            input_code(GPIO_BTN, false),
        )?;
        let v0 = analog.value(BTN_NODE).ok_or("no GPIO9 voltage in the solution")?;

        let origin = soc.hart.local_time();
        let mut cpu = Esp32c3Engine::new("esp32c3", soc);
        cpu.link_pin(PinLink {
            pin: GPIO_LED,
            target: 1,
        });
        cpu.link_pin(PinLink {
            pin: GPIO_BTN,
            target: 1,
        });
        cpu.link_input(GPIO_BTN, initial_input_level(v0, vdd_w));

        let mut kernel = Kernel::new(SimTime::from_us(50));
        let cpu_id = kernel.add_engine(Box::new(cpu));
        let analog_id = kernel.add_engine(Box::new(analog));
        debug_assert_eq!(cpu_id.0, 0);
        debug_assert_eq!(analog_id.0, 1);
        let stop = kernel.run_until(origin)?;
        if !matches!(stop, Stop::Reached(_)) {
            return Err(format!("ESP32-C3 stopped while connecting the circuit ({stop:?})").into());
        }

        let mut sim = Self {
            kind,
            kernel,
            cpu: cpu_id,
            analog: analog_id,
            board,
            closed: false,
            contact: false,
            bounce: BounceSettings::default(),
            pending_bounce: VecDeque::new(),
            next_sample: origin,
            origin,
            samples: TraceBuf::default_scope(),
            psu_samples: TraceBuf::default_scope(),
            sample_clock: SampleClock::new(0.0, trace::SAMPLE_STEP),
            supply,
            supply_mode: if supply.on { SupplyMode::Cv } else { SupplyMode::Off },
            supply_status: SupplyStatus::default(),
        };
        sim.record_sample()?;
        sim.sample_clock.take(sim.time());
        sim.next_sample = sim
            .origin
            .saturating_add(SimTime::from_secs_f64(sim.sample_clock.next().max(0.0)));
        Ok(sim)
    }

    #[allow(dead_code)]
    pub(super) fn kind(&self) -> ButtonKind {
        self.kind
    }

    pub(super) fn closed(&self) -> bool {
        self.closed
    }

    pub(super) fn time(&self) -> f64 {
        SimTime(self.kernel.now().0.saturating_sub(self.origin.0)).as_secs_f64()
    }

    pub(crate) fn set_scope_window(&mut self, visible: f64) {
        self.samples.set_visible_window(visible);
        self.psu_samples.set_visible_window(visible);
        self.sample_clock.set_step(self.samples.sample_step(), self.time());
        self.next_sample = self
            .origin
            .saturating_add(SimTime::from_secs_f64(self.sample_clock.next().max(0.0)));
    }

    pub(crate) fn advance(&mut self, seconds: f64) -> Result<(), Box<dyn Error>> {
        let until = self.origin.saturating_add(SimTime::from_secs_f64(seconds.max(0.0)));
        while self.kernel.now() < until {
            self.apply_due_bounce()?;
            let now = self.kernel.now();
            if now >= until {
                break;
            }
            let mut next = self.next_sample.min(until);
            if let Some(&(t, _)) = self.pending_bounce.front()
                && t < next
            {
                next = t;
            }
            if next > now {
                let stop = self.kernel.run_until(next)?;
                if !matches!(stop, Stop::Reached(_)) {
                    return Err(format!("ESP32-C3 stopped at {:.3} ms ({stop:?})", self.time() * 1e3).into());
                }
            }
            self.apply_due_bounce()?;
            if self.kernel.now() >= self.next_sample {
                if self.sample_clock.take(self.time()) {
                    self.record_sample()?;
                }
                self.next_sample = self
                    .origin
                    .saturating_add(SimTime::from_secs_f64(self.sample_clock.next().max(0.0)));
                if self.next_sample <= self.kernel.now() {
                    // Keep the bounce/run loop moving if the grid did not advance.
                    self.next_sample = self.kernel.now().saturating_add(SimTime::from_ns(1));
                }
            }
        }
        Ok(())
    }

    fn apply_due_bounce(&mut self) -> Result<(), Box<dyn Error>> {
        let now = self.kernel.now();
        while self.pending_bounce.front().is_some_and(|(t, _)| *t <= now) {
            let (_, closed) = self.pending_bounce.pop_front().expect("front");
            self.apply_contact(closed)?;
        }
        Ok(())
    }

    fn apply_contact(&mut self, closed: bool) -> Result<(), Box<dyn Error>> {
        self.contact = closed;
        let param = self
            .board
            .switch_param(SWITCH_REF, closed)
            .ok_or("SW1 is not a switch")?;
        let analog = self
            .kernel
            .engine_as::<AnalogEngine>(self.analog)
            .ok_or("analog solver is not registered")?;
        analog.set(&param.device, &param.param, param.value)?;
        Ok(())
    }

    fn record_sample(&mut self) -> Result<(), Box<dyn Error>> {
        let t = self.time();
        let ma = self.instant_ma()?;
        self.samples.push(t, ma);
        self.recycle_edges();
        let status = {
            let analog = self
                .kernel
                .engine_as::<AnalogEngine>(self.analog)
                .ok_or("analog solver is not registered")?;
            supply::regulate(analog, &self.supply, self.supply_mode)?
        };
        self.supply_mode = status.mode;
        self.supply_status = status;
        self.psu_samples.push(t, status.amps * 1e3);
        Ok(())
    }

    fn recycle_edges(&mut self) {
        if let Some(cpu) = self.kernel.engine_as::<Esp32c3Engine>(self.cpu) {
            cpu.recycle_consumed_edges();
        }
    }

    fn analog(&mut self) -> Result<&AnalogEngine, Box<dyn Error>> {
        self.kernel
            .engine_as::<AnalogEngine>(self.analog)
            .map(|engine| &*engine)
            .ok_or_else(|| "analog solver is not registered".into())
    }

    fn instant_ma(&mut self) -> Result<f64, Box<dyn Error>> {
        let analog = self.analog()?;
        let gpio = analog.value(GPIO_NODE).ok_or("no GPIO8 voltage in the solution")?;
        let anode = analog.value(ANODE_NODE).ok_or("no LED anode voltage in the solution")?;
        Ok((gpio - anode) * 1e3 / R_SERIES)
    }

    pub(super) fn average_ma(&self) -> f64 {
        average_window(&self.samples, self.time(), AVG_WINDOW)
    }

    pub(super) fn current_ma(&mut self) -> Result<f64, Box<dyn Error>> {
        self.instant_ma()
    }

    pub(super) fn button_volts(&mut self) -> Result<f64, Box<dyn Error>> {
        self.analog()?
            .value(BTN_NODE)
            .ok_or_else(|| "no GPIO9 voltage in the solution".into())
    }

    pub(super) fn trace(&self) -> Vec<(f64, f64)> {
        self.samples.to_vec()
    }

    pub(super) fn set_closed(&mut self, closed: bool) -> Result<(), Box<dyn Error>> {
        if self.closed == closed && self.pending_bounce.is_empty() {
            return Ok(());
        }
        self.closed = closed;
        self.pending_bounce.clear();
        let now = self.kernel.now();
        for (dt, c) in bounce_script(closed, self.bounce) {
            let at = now.saturating_add(SimTime::from_secs_f64(dt));
            if dt <= 0.0 {
                self.apply_contact(c)?;
            } else {
                self.pending_bounce.push_back((at, c));
            }
        }
        self.advance(self.time() + 1e-6)
    }

    pub(super) fn bounce(&self) -> BounceSettings {
        self.bounce
    }

    pub(super) fn set_bounce(&mut self, bounce: BounceSettings) {
        self.bounce = bounce;
    }

    #[allow(dead_code)]
    pub(super) fn supply(&self) -> SupplySettings {
        self.supply
    }

    pub(super) fn supply_status(&self) -> SupplyStatus {
        let mut status = self.supply_status;
        let led = self.samples.to_vec();
        let psu = self.psu_samples.to_vec();
        status.amps_avg = Some(
            crate::trace::average_periods(
                &psu,
                &led,
                self.time(),
                crate::trace::PWM_EDGE_HIGH,
                crate::trace::PWM_AVG_PERIODS,
                crate::trace::LED_AVG_WINDOW,
            ) * 1e-3,
        );
        status
    }

    pub(super) fn set_supply(&mut self, settings: SupplySettings) -> Result<(), Box<dyn Error>> {
        self.supply = settings.sanitized();
        let analog = self
            .kernel
            .engine_as::<AnalogEngine>(self.analog)
            .ok_or("analog solver is not registered")?;
        self.supply_mode = supply::apply(analog, &self.supply, self.supply_mode)?;
        self.advance(self.time() + 1e-6)?;
        self.record_sample()
    }

    pub(super) fn uart_bytes(&mut self) -> &[UartByte] {
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .uart_bytes()
    }

    pub(super) fn uart_baud(&mut self) -> Option<f64> {
        const APB_HZ: u64 = 80_000_000;
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .bus
            .uart0
            .baud(APB_HZ)
    }

    pub(super) fn push_uart_rx(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .push_uart_rx(bytes);
    }

    pub(crate) fn circuit_readout(&mut self) -> CircuitReadout {
        let sw = self.board.switches.get(SWITCH_REF).cloned();
        let mut out = CircuitReadout::new();
        {
            let analog = match self.analog() {
                Ok(a) => a,
                Err(_) => return out,
            };
            meters::take_nodes(analog, &["gpio8", "led_a", "vdd", "gpio9"], &mut out);
        }
        meters::add_led_series(&mut out, "R2", "D1", "gpio8", "led_a", R_SERIES);
        if let Some(s) = sw {
            let ohms = if self.contact { s.ron } else { s.roff };
            meters::add_resistor(&mut out, "SW1", "gpio9", "0", ohms);
        }
        meters::add_psu(&mut out, self.supply_status().display_amps());
        out
    }

    #[cfg(test)]
    pub(crate) fn trace_capacity(&self) -> usize {
        self.samples.capacity()
    }

    #[cfg(test)]
    pub(crate) fn perf_snapshot(&mut self) -> crate::stats::PerfSnapshot {
        let time = self.time();
        let samples = self.samples.len();
        crate::stats::PerfSnapshot::from_kernel(&mut self.kernel, self.cpu, self.analog, time, samples)
    }
}

fn average_window(samples: &TraceBuf, t: f64, window: f64) -> f64 {
    let pts = samples.to_vec();
    crate::trace::average_window(&pts, t, window)
}

#[cfg(test)]
impl ButtonSim {
    /// Start the edge log used by the parity goldens.
    pub(crate) fn parity_start(&mut self) {
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .start_edge_log();
    }

    /// Raw outputs for the parity goldens.
    pub(crate) fn parity_raw(&mut self) -> crate::parity::Raw {
        let origin = self.origin;
        let led = self.samples.to_vec();
        let psu = self.psu_samples.to_vec();
        let cpu = self
            .kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3");
        crate::parity::Raw {
            edges: cpu.logged_edges(),
            origin,
            uart: cpu.soc.uart_text(),
            led,
            psu,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maybe(kind: ButtonKind) -> Option<ButtonSim> {
        match ButtonSim::new(kind) {
            Ok(sim) => Some(sim),
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("fetch-esp-rom-elfs") {
                    assert!(
                        std::env::var_os("OF_REQUIRE_ESP_ROM").is_none(),
                        "OF_REQUIRE_ESP_ROM is set but the ROM was not found: {msg}"
                    );
                    None
                } else {
                    panic!("{msg}");
                }
            }
        }
    }

    #[test]
    fn polling_copies_the_button_to_the_led() {
        let Some(mut sim) = maybe(ButtonKind::Poll) else { return };
        sim.advance(200e-6).unwrap();
        assert!(
            sim.average_ma() < 0.2,
            "LED should be off at rest: {} mA",
            sim.average_ma()
        );
        let v_up = sim.button_volts().unwrap();
        assert!(v_up > 3.0, "pull-up should hold GPIO9 high: {v_up}");

        sim.set_closed(true).unwrap();
        sim.advance(sim.time() + 2e-3).unwrap();
        let inst = sim.current_ma().unwrap();
        let v_down = sim.button_volts().unwrap();
        assert!(
            inst > 1.0,
            "LED should be on while held: instant {inst} mA, average {} mA, GPIO9 {v_down} V",
            sim.average_ma()
        );
        assert!(v_down < 0.1, "closed button should pull GPIO9 down: {v_down}");
        let text = uart_text(&mut sim);
        // Default bounce is the canonical press train (5 edges), ending low.
        assert!(text.contains("poll: 5 low"), "{text:?}");
    }

    #[test]
    fn bounce_can_be_turned_off() {
        let Some(mut sim) = maybe(ButtonKind::Poll) else { return };
        sim.set_bounce(BounceSettings {
            enabled: false,
            seed: 0,
        });
        sim.advance(200e-6).unwrap();
        sim.set_closed(true).unwrap();
        sim.advance(sim.time() + 2e-3).unwrap();
        let text = uart_text(&mut sim);
        assert!(text.contains("poll: 1 low"), "{text:?}");
        assert!(!text.contains("poll: 5 low"), "{text:?}");
    }

    fn uart_text(sim: &mut ButtonSim) -> String {
        String::from_utf8_lossy(&sim.uart_bytes().iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned()
    }
}
