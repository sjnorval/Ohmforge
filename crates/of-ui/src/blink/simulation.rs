use std::error::Error;

use of_analog::{AnalogEngine, EngineConfig, Options};
use of_components::analog::{Board, Library, Placement, build};
use of_core::{EngineId, Kernel, SimTime, Stop};
use of_firmware::ElfImage;
use of_parts::parse_part;
use of_soc_esp32c3::{Esp32c3, Esp32c3Engine, PinLink, UartByte};

use crate::meters::{self, CircuitReadout};
use crate::pins::{add_pin_drivers, hook_pin};
use crate::supply::{self, SupplyMode, SupplySettings, SupplyStatus};
use crate::trace::{self, SAMPLE_STEP, TraceBuf};

#[cfg(test)]
pub(crate) const LED_ON_MA: f64 = 1.0;
const FIRMWARE: &[u8] = include_bytes!("../../../../firmware/esp32c3-blinky/blinky.elf");
const RESISTOR: &str = include_str!("../../../../parts/passives/resistor/part.toml");
const LED: &str = include_str!("../../../../parts/leds/red-0603/part.toml");
/// How far the stock unit test runs (the live GUI is not capped).
#[cfg(test)]
pub(crate) const END: SimTime = SimTime::from_us(2_600);
pub(crate) const R_SERIES: f64 = 330.0;
pub(crate) const GPIO: u8 = 8;
pub(crate) const RESISTOR_REF: &str = "R1";
pub(crate) const LED_REF: &str = "D1";
const GPIO_NODE: &str = "v(gpio8)";
const ANODE_NODE: &str = "v(led_a)";

pub(crate) struct Blink {
    kernel: Kernel,
    cpu: EngineId,
    analog: EngineId,
    samples: TraceBuf,
    supply: SupplySettings,
    supply_mode: SupplyMode,
    supply_status: SupplyStatus,
}

impl Blink {
    #[allow(dead_code)]
    pub(crate) fn new() -> Result<Self, Box<dyn Error>> {
        Self::with_supply(SupplySettings::default())
    }

    pub(crate) fn with_supply(supply: SupplySettings) -> Result<Self, Box<dyn Error>> {
        let supply = supply.sanitized();
        let mut lib = Library::new();
        lib.add(parse_part(RESISTOR)?, None)?;
        lib.add(parse_part(LED)?, None)?;
        let board = Board {
            placements: vec![
                Placement::new(RESISTOR_REF, "generic.resistor", &[("1", "gpio8"), ("2", "led_a")])
                    .with("resistance", R_SERIES),
                Placement::new(LED_REF, "generic.led-red-0603", &[("A", "led_a"), ("K", "0")]),
            ],
            ..Board::default()
        };
        let mut circuit = build(&lib, &board)?.circuit;
        let pin = circuit.node("gpio8");
        let vdd = circuit.node("vdd");
        supply::add_supply(&mut circuit, vdd, &supply)?;
        let mut soc = Esp32c3::new();
        soc.load_elf(&ElfImage::parse(FIRMWARE)?)?;
        add_pin_drivers(&mut circuit, GPIO, pin, vdd, soc.pin(usize::from(GPIO)))?;

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
        hook_pin(&mut analog, GPIO);

        // Register the CPU first so its GPIO changes reach the analog solver in time.
        let mut kernel = Kernel::new(SimTime::from_us(50));
        let cpu = kernel.add_engine(Box::new(Esp32c3Engine::new("esp32c3", soc)));
        let analog_id = kernel.add_engine(Box::new(analog));
        kernel
            .engine_as::<Esp32c3Engine>(cpu)
            .expect("registered ESP32-C3")
            .link_pin(PinLink {
                pin: GPIO,
                target: analog_id.0,
            });
        let mut sim = Self {
            kernel,
            cpu,
            analog: analog_id,
            samples: TraceBuf::default_scope(),
            supply,
            supply_mode: if supply.on { SupplyMode::Cv } else { SupplyMode::Off },
            supply_status: SupplyStatus::default(),
        };
        sim.record_sample()?;
        Ok(sim)
    }

    pub(crate) fn advance(&mut self, seconds: f64) -> Result<(), Box<dyn Error>> {
        let until = SimTime::from_secs_f64(seconds.max(0.0));
        while self.kernel.now() < until {
            let next = self
                .kernel
                .now()
                .saturating_add(SimTime::from_secs_f64(SAMPLE_STEP))
                .min(until);
            let stop = self.kernel.run_until(next)?;
            if !matches!(stop, Stop::Reached(_)) {
                return Err(format!("ESP32-C3 stopped at {:.3} ms ({stop:?})", self.time() * 1e3).into());
            }
            self.record_sample()?;
        }
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
        Ok(())
    }

    pub(crate) fn time(&self) -> f64 {
        self.kernel.now().as_secs_f64()
    }

    #[allow(dead_code)]
    pub(crate) fn supply(&self) -> SupplySettings {
        self.supply
    }

    pub(crate) fn supply_status(&self) -> SupplyStatus {
        self.supply_status
    }

    pub(crate) fn set_supply(&mut self, settings: SupplySettings) -> Result<(), Box<dyn Error>> {
        self.supply = settings.sanitized();
        let analog = self
            .kernel
            .engine_as::<AnalogEngine>(self.analog)
            .ok_or("analog solver is not registered")?;
        self.supply_mode = supply::apply(analog, &self.supply, self.supply_mode)?;
        // A short step so the rail updates while playback is paused.
        self.advance(self.time() + 1e-6)
    }

    #[allow(dead_code)] // unit tests; UI syncs via `uart_bytes`.
    pub(crate) fn uart(&mut self) -> String {
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .uart_text()
    }

    pub(crate) fn uart_bytes(&mut self) -> &[UartByte] {
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .uart_bytes()
    }

    pub(crate) fn uart_baud(&mut self) -> Option<f64> {
        const APB_HZ: u64 = 80_000_000;
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .bus
            .uart0
            .baud(APB_HZ)
    }

    /// Queue host bytes for UART0 RX; they arrive at the configured baud.
    pub(crate) fn push_uart_rx(&mut self, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        self.kernel
            .engine_as::<Esp32c3Engine>(self.cpu)
            .expect("registered ESP32-C3")
            .soc
            .push_uart_rx(bytes);
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
        Ok(current_ma(gpio, anode))
    }

    pub(crate) fn current_ma(&mut self) -> Result<f64, Box<dyn Error>> {
        self.instant_ma()
    }

    /// Time-averaged LED current over the last 2 ms, for display only.
    pub(crate) fn average_ma(&self) -> f64 {
        trace::average_window(&self.samples.to_vec(), self.time(), trace::LED_AVG_WINDOW)
    }

    pub(crate) fn trace(&self) -> Vec<(f64, f64)> {
        self.samples.to_vec()
    }

    pub(crate) fn pwm_duty_freq(&self) -> (Option<f64>, Option<f64>) {
        let pts: Vec<_> = self.samples.to_vec();
        trace::pwm_duty_freq(&pts, self.time(), 0.5)
    }

    pub(crate) fn circuit_readout(&mut self) -> CircuitReadout {
        let mut out = CircuitReadout::new();
        {
            let analog = match self.analog() {
                Ok(a) => a,
                Err(_) => return out,
            };
            meters::take_nodes(analog, &["gpio8", "led_a", "vdd"], &mut out);
        }
        meters::add_led_series(&mut out, "R1", "D1", "gpio8", "led_a", R_SERIES);
        meters::add_psu(&mut out, self.supply_status.amps);
        out
    }

    #[cfg(test)]
    pub(crate) fn perf_snapshot(&mut self) -> crate::stats::PerfSnapshot {
        let time = self.time();
        let samples = self.samples.len();
        crate::stats::PerfSnapshot::from_kernel(&mut self.kernel, self.cpu, self.analog, time, samples)
    }
}

fn current_ma(gpio: f64, anode: f64) -> f64 {
    (gpio - anode) * 1e3 / R_SERIES
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_blink_advances_firmware_and_records_led_current() {
        let mut sim = Blink::new().unwrap();
        assert_eq!(sim.time(), 0.0);
        assert!(sim.uart().is_empty());

        sim.advance(125e-6).unwrap();
        assert!((3.0..4.5).contains(&sim.current_ma().unwrap()));
        assert_eq!(sim.uart(), "ohmforge blinky\n");

        sim.advance(375e-6).unwrap();
        assert!(sim.current_ma().unwrap().abs() < 1e-3);
        sim.advance(625e-6).unwrap();
        assert!((3.0..4.5).contains(&sim.current_ma().unwrap()));

        sim.advance(END.as_secs_f64()).unwrap();
        let trace = sim.trace();
        assert!(
            trace.iter().any(|&(_, ma)| ma > LED_ON_MA),
            "LED never turned on: {trace:?}"
        );
        assert!(
            trace.iter().any(|&(_, ma)| ma.abs() < 1e-3),
            "LED never turned off: {trace:?}"
        );
        assert!((trace.last().unwrap().0 - END.as_secs_f64()).abs() < 1e-9);
    }

    #[test]
    fn monitor_receives_blink_banner_when_synced() {
        let mut demo_monitor = of_instruments::SerialMonitor::new();
        let mut sim = Blink::new().unwrap();
        sim.advance(125e-6).unwrap();
        for b in sim.uart_bytes() {
            demo_monitor.push_rx(b.at, &[b.byte]);
        }
        assert_eq!(demo_monitor.text_device_to_host(), "ohmforge blinky\n");
    }

    #[test]
    fn push_uart_rx_is_readable_from_fifo() {
        let mut sim = Blink::new().unwrap();
        sim.push_uart_rx(b"Z");
        // 8N1 at a typical baud is ~87 µs per byte; wait a couple of frames.
        let baud = sim.uart_baud().unwrap_or(115_200.0).max(1.0);
        sim.advance(sim.time() + 20.0 / baud).unwrap();
        let engine = sim
            .kernel
            .engine_as::<Esp32c3Engine>(sim.cpu)
            .expect("registered ESP32-C3");
        assert_eq!(engine.soc.bus.uart0.pop_rx(), b'Z');
    }

    #[test]
    fn led_current_scales_with_supply_voltage() {
        let mut sim = Blink::new().unwrap();
        sim.advance(125e-6).unwrap();
        let i33 = sim.current_ma().unwrap();
        assert!((3.0..4.5).contains(&i33), "{i33}");

        let hi = SupplySettings {
            volts: 5.0,
            ..SupplySettings::default()
        };
        sim.set_supply(hi).unwrap();
        // Stay in the on period (~0–250 µs).
        let i50 = sim.current_ma().unwrap();
        assert!(
            i50 > i33 * 1.15,
            "expected higher LED current at 5 V: 3.3 V {i33} mA, 5 V {i50} mA"
        );
        assert!(sim.supply_status().outside_rated);
    }

    #[test]
    fn current_limit_clamps_the_led() {
        let settings = SupplySettings {
            current_limit: 0.5e-3,
            ..SupplySettings::default()
        };
        let mut sim = Blink::with_supply(settings).unwrap();
        sim.advance(125e-6).unwrap();
        let status = sim.supply_status();
        let ma = sim.current_ma().unwrap();
        assert!(
            status.mode == SupplyMode::Cc || ma < LED_ON_MA,
            "limit did not clamp: status={status:?} led={ma} mA"
        );
        assert!(status.amps <= 0.6e-3 + 1e-5, "{status:?}");
    }
}
