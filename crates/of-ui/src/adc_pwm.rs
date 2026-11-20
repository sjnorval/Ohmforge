//! Potentiometer dimmer and LED fade simulations for the desktop demos.

mod schematic;

use std::collections::VecDeque;
use std::error::Error;
use std::sync::OnceLock;

use of_analog::{AnalogEngine, EngineConfig, Options};
use of_components::analog::{Board, BuiltBoard, Library, Placement, build};
use of_core::{EngineId, Kernel, SimTime, Stop};
use of_parts::parse_part;
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, PinLink, UartByte, adc_request_code, adc_value_code};

use crate::pins::{add_pin_drivers, hook_pin};
use crate::rom;
use crate::supply::{self, SupplyMode, SupplySettings, SupplyStatus};

pub(super) use schematic::show as show_schematic;

const PARTITIONS: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/partitions.bin");
const DIMMER: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/pot_dimmer.bin");
const FADE: &[u8] = include_bytes!("../../../firmware/esp32c3-esp-hal-adc-pwm/led_fade.bin");
const RESISTOR: &str = include_str!("../../../parts/passives/resistor/part.toml");
const LED: &str = include_str!("../../../parts/leds/red-0603/part.toml");
const POT: &str = include_str!("../../../parts/passives/bourns-ptv09a/part.toml");

#[cfg_attr(not(test), allow(dead_code))]
const VDD: f64 = 3.3;
const R_SERIES: f64 = 330.0;
pub(super) const GPIO_LED: u8 = 8;
pub(super) const GPIO_POT: u8 = 2;
pub(super) const RESISTOR_REF: &str = "R2";
pub(super) const LED_REF: &str = "D1";
pub(super) const POT_REF: &str = "RV1";
const GPIO_NODE: &str = "v(gpio8)";
const ANODE_NODE: &str = "v(led_a)";
const WIPER_NODE: &str = "v(gpio2)";

const DIMMER_BANNER: &str = "ohmforge esp-hal pot dimmer\n";
const FADE_BANNER: &str = "ohmforge esp-hal led fade\n";

/// How long a display-average window lasts (2 ms: two 1 kHz periods, ten 5 kHz periods).
const AVG_WINDOW: f64 = 2e-3;
/// Trace and sample history kept in memory.
const TRACE_WINDOW: f64 = 0.16;
/// Analog samples taken this often so the on-screen LED can be averaged.
const SAMPLE_STEP: SimTime = SimTime::from_us(50);

static DIMMER_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();
static FADE_SOC: OnceLock<Result<Esp32c3, String>> = OnceLock::new();

/// Which committed esp-hal app to boot.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum PwmKind {
    Dimmer,
    Fade,
}

impl PwmKind {
    fn banner(self) -> &'static str {
        match self {
            Self::Dimmer => DIMMER_BANNER,
            Self::Fade => FADE_BANNER,
        }
    }

    fn app(self) -> &'static [u8] {
        match self {
            Self::Dimmer => DIMMER,
            Self::Fade => FADE,
        }
    }

    fn cache(self) -> &'static OnceLock<Result<Esp32c3, String>> {
        match self {
            Self::Dimmer => &DIMMER_SOC,
            Self::Fade => &FADE_SOC,
        }
    }
}

/// Cached boot result, if the ROM has already been tried for this app.
pub(super) fn cached(kind: PwmKind) -> Option<&'static Result<Esp32c3, String>> {
    kind.cache().get()
}

/// Boot the app through the fetched ROM (cached after the first success or error).
pub(super) fn booted(kind: PwmKind) -> Result<Esp32c3, String> {
    kind.cache()
        .get_or_init(|| rom::boot_app(kind.app(), PARTITIONS, kind.banner()))
        .clone()
}

pub(super) struct PwmSim {
    kind: PwmKind,
    kernel: Kernel,
    cpu: EngineId,
    analog: EngineId,
    board: BuiltBoard,
    /// Wiper position, 0 at pin 1 (ground) to 1 at pin 3 (VDD).
    position: f64,
    /// Kernel time at which the circuit was connected.
    origin: SimTime,
    /// Instantaneous LED current samples (t relative to `origin`, mA).
    samples: VecDeque<(f64, f64)>,
    supply: SupplySettings,
    supply_mode: SupplyMode,
    supply_status: SupplyStatus,
}

impl PwmSim {
    pub(super) fn new(kind: PwmKind, position: f64) -> Result<Self, Box<dyn Error>> {
        Self::with_supply(kind, position, SupplySettings::default())
    }

    pub(super) fn with_supply(kind: PwmKind, position: f64, supply: SupplySettings) -> Result<Self, Box<dyn Error>> {
        let soc = booted(kind)?;
        Self::from_soc(kind, soc, position, supply)
    }

    fn from_soc(kind: PwmKind, soc: Esp32c3, position: f64, supply: SupplySettings) -> Result<Self, Box<dyn Error>> {
        let supply = supply.sanitized();
        let position = position.clamp(0.0, 1.0);
        let mut lib = Library::new();
        lib.add(parse_part(RESISTOR)?, None)?;
        lib.add(parse_part(LED)?, None)?;
        lib.add(parse_part(POT)?, None)?;
        let mut placements = vec![
            Placement::new(RESISTOR_REF, "generic.resistor", &[("1", "gpio8"), ("2", "led_a")])
                .with("resistance", R_SERIES),
            Placement::new(LED_REF, "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
        ];
        if kind == PwmKind::Dimmer {
            placements.insert(
                0,
                Placement::new(
                    POT_REF,
                    "bourns.ptv09a-b103",
                    &[("1", "0"), ("W", "gpio2"), ("3", "vdd")],
                )
                .with("position", position),
            );
        }
        let mut board = build(
            &lib,
            &Board {
                placements,
                ..Board::default()
            },
        )?;
        let vdd = board.circuit.node("vdd");
        supply::add_supply(&mut board.circuit, vdd, &supply)?;
        let node = board.circuit.node("gpio8");
        add_pin_drivers(&mut board.circuit, GPIO_LED, node, vdd, soc.pin(usize::from(GPIO_LED)))?;

        let circuit = std::mem::replace(&mut board.circuit, of_analog::Circuit::new());
        let mut analog = AnalogEngine::new(
            "analog",
            circuit,
            Options::default(),
            EngineConfig {
                max_step: 20e-6,
                record: false,
                ..EngineConfig::default()
            },
        )?;
        hook_pin(&mut analog, GPIO_LED);
        if kind == PwmKind::Dimmer {
            analog.sample_on(adc_request_code(GPIO_POT), WIPER_NODE, 0, adc_value_code(GPIO_POT))?;
        }

        let origin = soc.hart.local_time();
        let mut cpu = Esp32c3Engine::new("esp32c3", soc);
        cpu.link_pin(PinLink {
            pin: GPIO_LED,
            target: 1,
        });
        if kind == PwmKind::Dimmer {
            cpu.link_adc(GPIO_POT, 1);
        }

        let mut kernel = Kernel::new(SimTime::from_us(50));
        let cpu_id = kernel.add_engine(Box::new(cpu));
        let analog_id = kernel.add_engine(Box::new(analog));
        debug_assert_eq!(cpu_id.0, 0);
        debug_assert_eq!(analog_id.0, 1);
        // Catch the analog solver up to the already-booted CPU without showing
        // those idle microseconds on the trace.
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
            position,
            origin,
            samples: VecDeque::new(),
            supply,
            supply_mode: if supply.on { SupplyMode::Cv } else { SupplyMode::Off },
            supply_status: SupplyStatus::default(),
        };
        sim.record_sample()?;
        Ok(sim)
    }

    pub(super) fn kind(&self) -> PwmKind {
        self.kind
    }

    pub(super) fn position(&self) -> f64 {
        self.position
    }

    pub(super) fn time(&self) -> f64 {
        SimTime(self.kernel.now().0.saturating_sub(self.origin.0)).as_secs_f64()
    }

    pub(super) fn advance(&mut self, seconds: f64) -> Result<(), Box<dyn Error>> {
        let until = self.origin.saturating_add(SimTime::from_secs_f64(seconds.max(0.0)));
        while self.kernel.now() < until {
            let next = self.kernel.now().saturating_add(SAMPLE_STEP).min(until);
            let stop = self.kernel.run_until(next)?;
            if !matches!(stop, Stop::Reached(_)) {
                return Err(format!("ESP32-C3 stopped at {:.3} ms ({stop:?})", self.time() * 1e3).into());
            }
            self.record_sample()?;
        }
        self.trim();
        Ok(())
    }

    fn record_sample(&mut self) -> Result<(), Box<dyn Error>> {
        let t = self.time();
        let ma = self.instant_ma()?;
        self.samples.push_back((t, ma));
        let status = {
            let analog = self
                .kernel
                .engine_as::<AnalogEngine>(self.analog)
                .ok_or("analog solver is not registered")?;
            supply::regulate(analog, &self.supply, self.supply_mode)?
        };
        self.supply_mode = status.mode;
        self.supply_status = status;
        Ok(())
    }

    fn trim(&mut self) {
        let t = self.time();
        while self.samples.len() > 1 && t - self.samples[0].0 > TRACE_WINDOW {
            self.samples.pop_front();
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

    /// Average LED current over the last [`AVG_WINDOW`], for display only.
    pub(super) fn average_ma(&self) -> f64 {
        average_window(&self.samples, self.time(), AVG_WINDOW)
    }

    pub(super) fn wiper_volts(&mut self) -> Result<f64, Box<dyn Error>> {
        self.analog()?
            .value(WIPER_NODE)
            .ok_or_else(|| "no wiper voltage in the solution".into())
    }

    pub(super) fn trace(&self) -> Vec<(f64, f64)> {
        // A moving average of the raw samples, so 1 kHz / 5 kHz PWM is a
        // brightness line rather than a blur of edges.
        let mut out = Vec::new();
        for i in 0..self.samples.len() {
            let t = self.samples[i].0;
            out.push((t, average_window(&self.samples, t, AVG_WINDOW)));
        }
        out
    }

    pub(super) fn set_position(&mut self, position: f64) -> Result<(), Box<dyn Error>> {
        let position = position.clamp(0.0, 1.0);
        self.position = position;
        let params = self
            .board
            .pot_params(POT_REF, position)
            .ok_or("RV1 is not a potentiometer")?;
        {
            let analog = self
                .kernel
                .engine_as::<AnalogEngine>(self.analog)
                .ok_or("analog solver is not registered")?;
            for p in params {
                analog.set(&p.device, &p.param, p.value)?;
            }
        }
        // One short step so the wiper voltage updates while playback is paused.
        self.advance(self.time() + 1e-6)
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn supply_status(&self) -> SupplyStatus {
        self.supply_status
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub(super) fn set_supply(&mut self, settings: SupplySettings) -> Result<(), Box<dyn Error>> {
        self.supply = settings.sanitized();
        let analog = self
            .kernel
            .engine_as::<AnalogEngine>(self.analog)
            .ok_or("analog solver is not registered")?;
        self.supply_mode = supply::apply(analog, &self.supply, self.supply_mode)?;
        self.advance(self.time() + 1e-6)
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
}

fn average_window(samples: &VecDeque<(f64, f64)>, t: f64, window: f64) -> f64 {
    let start = t - window;
    let pts: Vec<(f64, f64)> = samples
        .iter()
        .copied()
        .filter(|(tt, _)| *tt >= start && *tt <= t)
        .collect();
    match pts.len() {
        0 => 0.0,
        1 => pts[0].1,
        _ => {
            let mut area = 0.0;
            for w in pts.windows(2) {
                area += (w[0].1 + w[1].1) * 0.5 * (w[1].0 - w[0].0);
            }
            let span = pts.last().unwrap().0 - pts[0].0;
            if span <= 0.0 {
                pts.last().unwrap().1
            } else {
                area / span
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn maybe_dimmer(pos: f64) -> Option<PwmSim> {
        match PwmSim::new(PwmKind::Dimmer, pos) {
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

    fn maybe_fade() -> Option<PwmSim> {
        match PwmSim::new(PwmKind::Fade, 0.5) {
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
    fn dimmer_reads_the_pot_and_the_slider_changes_the_duty() {
        let Some(mut sim) = maybe_dimmer(0.3) else { return };
        assert!((sim.wiper_volts().unwrap() - 0.3 * VDD).abs() < 0.05);
        sim.advance(15e-3).unwrap();
        let mid = uart_readings(&mut sim);
        assert!(
            mid.iter()
                .any(|&(mv, _)| (f64::from(mv) - 0.3 * VDD * 1e3).abs() < 80.0),
            "{mid:?}"
        );
        sim.set_position(0.85).unwrap();
        let after_turn = uart_readings(&mut sim).len();
        sim.advance(20e-3).unwrap();
        let high = uart_readings(&mut sim);
        assert!(high.len() > after_turn, "{high:?}");
        assert!(
            high.iter()
                .any(|&(mv, duty)| (f64::from(mv) - 0.85 * VDD * 1e3).abs() < 80.0 && duty > 800),
            "{high:?}"
        );
        assert!(sim.average_ma() > 1.0, "average {} mA", sim.average_ma());
        assert!(sim.trace().len() > 10);
    }

    #[test]
    fn fade_ramps_the_display_average() {
        let Some(mut sim) = maybe_fade() else { return };
        sim.advance(20e-3).unwrap();
        let early = sim.average_ma();
        sim.advance(40e-3).unwrap();
        let later = sim.average_ma();
        // The hardware fade climbs for ~51 ms; 20 ms to 60 ms should get brighter.
        assert!(later > early + 0.3, "early {early} mA, later {later} mA");
        let text = String::from_utf8_lossy(&sim.uart_bytes().iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned();
        assert!(text.contains("ohmforge esp-hal led fade"), "{text:?}");
    }

    fn uart_readings(sim: &mut PwmSim) -> Vec<(u32, u32)> {
        let text = String::from_utf8_lossy(&sim.uart_bytes().iter().map(|b| b.byte).collect::<Vec<_>>()).into_owned();
        text.lines()
            .filter_map(|l| {
                let rest = l.strip_prefix("adc: ")?;
                let (mv, duty) = rest.split_once(" duty: ")?;
                Some((mv.parse().ok()?, duty.parse().ok()?))
            })
            .collect()
    }

    #[test]
    fn pot_divider_scales_with_supply_voltage() {
        let Some(mut sim) = maybe_dimmer(0.5) else { return };
        let v33 = sim.wiper_volts().unwrap();
        assert!((v33 - 0.5 * VDD).abs() < 0.05, "{v33}");

        let hi = SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        };
        sim.set_supply(hi).unwrap();
        let v50 = sim.wiper_volts().unwrap();
        assert!(
            v50 > v33 * 1.2,
            "wiper should scale with the rail: 3.3 V {v33} V, 5 V {v50} V"
        );
        assert!(sim.supply_status().outside_rated);
    }

    #[test]
    fn current_limit_clamps_led_current() {
        let settings = SupplySettings {
            current_limit: 0.5e-3,
            ..SupplySettings::default()
        };
        let mut sim = match PwmSim::with_supply(PwmKind::Dimmer, 1.0, settings) {
            Ok(sim) => sim,
            Err(e) => {
                let msg = e.to_string();
                if msg.contains("fetch-esp-rom-elfs") {
                    assert!(
                        std::env::var_os("OF_REQUIRE_ESP_ROM").is_none(),
                        "OF_REQUIRE_ESP_ROM is set but the ROM was not found: {msg}"
                    );
                    return;
                }
                panic!("{msg}");
            }
        };
        sim.advance(15e-3).unwrap();
        let status = sim.supply_status();
        let avg = sim.average_ma();
        assert!(
            status.mode == SupplyMode::Cc || avg < 1.0,
            "limit did not clamp: status={status:?} led={avg} mA"
        );
        assert!(status.amps <= 0.6e-3 + 1e-5, "{status:?}");
    }
}
