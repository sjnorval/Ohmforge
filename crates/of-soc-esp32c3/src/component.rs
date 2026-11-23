//! ESP32-C3 as an engine [`Component`] (RFC 0001 / design §3c).

use std::collections::BTreeMap;

use of_analog::devices::Resistor;
use of_analog::{AnalogEngine, Edge};
use of_component_api::{Component, ComponentInfo, ElabCtx, ElabError, PinKind, PinSpec, SolutionView};
use of_core::Engine;
use of_firmware::ElfImage;

use crate::engine::{Esp32c3Engine, PinLink, adc_request_code, adc_value_code, initial_input_level, input_code};
use crate::gpio::{GPIO_COUNT, PinOut, VIH_FRACTION, VIL_FRACTION};
use crate::pad::{add_pin_drivers, hook_pin};
use crate::soc::Esp32c3;

/// Quiescent current estimate drawn from 3V3 (A); datasheet modem-sleep order.
pub const QUIESCENT_A: f64 = 5e-3;

/// Port name for UART0 (TX/RX) that a UART console instrument can attach to.
pub const UART0_PORT: &str = "uart0";

/// How the component loads firmware when [`Component::take_engine`] runs.
#[derive(Debug, Clone)]
pub enum BootImage {
    /// Direct ELF load ([`Esp32c3::load_elf`]).
    Elf(Vec<u8>),
}

/// ESP32-C3 chip component: supply pins, selected GPIOs, and an active CPU engine.
pub struct Esp32c3Component {
    info: ComponentInfo,
    boot: Option<BootImage>,
    /// GPIO numbers elaborated (from connected `GPIOn` pins).
    gpios: Vec<u8>,
    /// Pads that should take their level from the analog circuit (inputs).
    input_pins: Vec<u8>,
    /// ADC pads among connected GPIOs (0–5).
    adc_pads: Vec<u8>,
    /// Kernel event target for pin/ADC events (set before [`Self::take_engine`]).
    analog_target: u32,
    /// Net name for 3V3 after elaborate.
    vdd_net: Option<String>,
    /// GPIO number → circuit net name, for ADC `sample_on` and input watches.
    gpio_nets: BTreeMap<u8, String>,
}

impl Esp32c3Component {
    /// Place the chip as `refdes` with a direct-ELF image and the listed pins.
    ///
    /// `pins` must include `3V3` and `GND`, plus each `GPIOn` that is wired
    /// (only connected pins are declared — design §3c).
    pub fn with_elf(refdes: &str, elf: &[u8], pins: &[&str]) -> Result<Self, ElabError> {
        let mut specs = Vec::new();
        let mut saw_vdd = false;
        let mut saw_gnd = false;
        for &name in pins {
            match name {
                "3V3" => {
                    saw_vdd = true;
                    specs.push(PinSpec {
                        name: "3V3".into(),
                        kind: PinKind::Power,
                    });
                }
                "GND" => {
                    saw_gnd = true;
                    specs.push(PinSpec {
                        name: "GND".into(),
                        kind: PinKind::Ground,
                    });
                }
                other => {
                    let n =
                        parse_gpio(other).ok_or_else(|| ElabError::msg(format!("{refdes}: unknown pin '{other}'")))?;
                    if usize::from(n) >= GPIO_COUNT {
                        return Err(ElabError::msg(format!("{refdes}: GPIO{n} out of range")));
                    }
                    specs.push(PinSpec {
                        name: format!("GPIO{n}"),
                        kind: PinKind::Bidir,
                    });
                }
            }
        }
        if !saw_vdd || !saw_gnd {
            return Err(ElabError::msg(format!(
                "{refdes}: Esp32c3Component needs 3V3 and GND pins"
            )));
        }
        Ok(Self {
            info: ComponentInfo {
                part_id: "espressif.esp32-c3".into(),
                refdes: refdes.to_owned(),
                pins: specs,
            },
            boot: Some(BootImage::Elf(elf.to_vec())),
            gpios: Vec::new(),
            input_pins: Vec::new(),
            adc_pads: Vec::new(),
            analog_target: 1,
            vdd_net: None,
            gpio_nets: BTreeMap::new(),
        })
    }

    /// Kernel id of the analog engine that receives pin/ADC events (default 1).
    pub fn set_analog_target(&mut self, target: u32) {
        self.analog_target = target;
    }

    /// Treat `pin` as a circuit-driven input (V_IH/V_IL watches + [`Esp32c3Engine::link_input`]).
    pub fn enable_input(&mut self, pin: u8) {
        if !self.input_pins.contains(&pin) {
            self.input_pins.push(pin);
        }
    }

    /// GPIO numbers that were connected at elaboration.
    pub fn connected_gpios(&self) -> &[u8] {
        &self.gpios
    }

    /// Net name for the chip's `3V3` pin after elaboration.
    pub fn vdd_net(&self) -> Option<&str> {
        self.vdd_net.as_deref()
    }

    /// Circuit net for a connected GPIO, after elaboration.
    pub fn gpio_net(&self, pin: u8) -> Option<&str> {
        self.gpio_nets.get(&pin).map(String::as_str)
    }

    /// UART0 port name for a serial console instrument.
    pub fn uart0_port(&self) -> &'static str {
        UART0_PORT
    }

    /// Wire GPIO pad codes into `analog` for every connected GPIO.
    pub fn hook_gpio_outputs(&self, analog: &mut AnalogEngine) {
        for &pin in &self.gpios {
            hook_pin(analog, &self.info.refdes, pin);
        }
    }

    /// Register V_IH / V_IL watches relative to this chip's `3V3` for enabled inputs.
    pub fn hook_gpio_inputs(&self, analog: &mut AnalogEngine, cpu_target: u32) -> Result<(), String> {
        let vdd = self
            .vdd_net
            .as_deref()
            .ok_or_else(|| "Esp32c3Component: not elaborated".to_owned())?;
        let vdd_sig = format!("v({vdd})");
        for &pin in &self.input_pins {
            let net = self
                .gpio_nets
                .get(&pin)
                .ok_or_else(|| format!("Esp32c3Component: GPIO{pin} not connected"))?;
            let sig = format!("v({net})");
            analog
                .watch_relative(
                    &sig,
                    &vdd_sig,
                    VIH_FRACTION,
                    Edge::Rising,
                    cpu_target,
                    input_code(pin, true),
                )
                .map_err(|e| e.to_string())?;
            analog
                .watch_relative(
                    &sig,
                    &vdd_sig,
                    VIL_FRACTION,
                    Edge::Falling,
                    cpu_target,
                    input_code(pin, false),
                )
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Register ADC sample points for connected ADC pads (GPIO0–4 / pad 0–4, GPIO5 = ADC2).
    pub fn hook_adc(&self, analog: &mut AnalogEngine) -> Result<(), String> {
        for &pad in &self.adc_pads {
            let Some(net) = self.gpio_nets.get(&pad) else {
                continue;
            };
            analog
                .sample_on(adc_request_code(pad), &format!("v({net})"), 0, adc_value_code(pad))
                .map_err(|e| e.to_string())?;
        }
        Ok(())
    }

    /// Build the CPU engine with pin/ADC/input links aimed at [`Self::analog_target`].
    pub fn build_engine(&mut self, analog: &AnalogEngine) -> Result<Esp32c3Engine, ElabError> {
        let boot = self
            .boot
            .take()
            .ok_or_else(|| ElabError::msg("Esp32c3Component: firmware already consumed"))?;
        let mut soc = Esp32c3::new();
        match boot {
            BootImage::Elf(bytes) => {
                let img = ElfImage::parse(&bytes).map_err(|e| ElabError::msg(e.to_string()))?;
                soc.load_elf(&img).map_err(|e| ElabError::msg(e.to_string()))?;
            }
        }
        let mut eng = Esp32c3Engine::new(&self.info.refdes, soc);
        for &pin in &self.gpios {
            eng.link_pin(PinLink {
                pin,
                target: self.analog_target,
            });
        }
        for &pad in &self.adc_pads {
            eng.link_adc(pad, self.analog_target);
        }
        if let Some(vdd_net) = &self.vdd_net {
            let vdd = analog.value(&format!("v({vdd_net})")).unwrap_or(3.3);
            for &pin in &self.input_pins {
                let net = self.gpio_nets.get(&pin).map(String::as_str).unwrap_or("gnd");
                let v = analog.value(&format!("v({net})")).unwrap_or(0.0);
                eng.link_input(pin, initial_input_level(v, vdd.max(0.1)));
            }
        }
        Ok(eng)
    }
}

fn parse_gpio(name: &str) -> Option<u8> {
    name.strip_prefix("GPIO")
        .or_else(|| name.strip_prefix("gpio"))
        .and_then(|s| s.parse().ok())
}

fn net_of(circuit: &of_analog::Circuit, node: of_analog::Var) -> String {
    circuit
        .vars()
        .get(node)
        .map(|v| {
            v.name
                .strip_prefix("v(")
                .and_then(|s| s.strip_suffix(')'))
                .unwrap_or("gnd")
                .to_owned()
        })
        .unwrap_or_else(|| "gnd".into())
}

impl Component for Esp32c3Component {
    fn info(&self) -> &ComponentInfo {
        &self.info
    }

    fn elaborate(&mut self, cx: &mut dyn ElabCtx) -> Result<(), ElabError> {
        let vdd = cx.pin_node("3V3")?;
        let gnd = cx.pin_node("GND")?;
        self.vdd_net = Some(net_of(cx.circuit_mut(), vdd));
        self.gpios.clear();
        self.adc_pads.clear();
        self.gpio_nets.clear();

        let gpio_pins: Vec<(u8, String)> = self
            .info
            .pins
            .iter()
            .filter_map(|p| parse_gpio(&p.name).map(|n| (n, p.name.clone())))
            .collect();

        for (num, name) in gpio_pins {
            let gpio = cx.pin_node(&name)?;
            let net = net_of(cx.circuit_mut(), gpio);
            add_pin_drivers(cx.circuit_mut(), &self.info.refdes, num, gpio, vdd, gnd, PinOut::HIZ)
                .map_err(ElabError::msg)?;
            self.gpios.push(num);
            self.gpio_nets.insert(num, net);
            if num <= 5 {
                self.adc_pads.push(num);
            }
        }

        let r_iq = 3.3 / QUIESCENT_A;
        let iq = format!("{}_iq", self.info.refdes.to_ascii_lowercase());
        cx.add_device(Box::new(Resistor::new(&iq, vdd, gnd, r_iq)))?;
        Ok(())
    }

    fn terminal_currents(&self, view: &dyn SolutionView, out: &mut [f64]) {
        out.fill(0.0);
        if out.len() < 2 {
            return;
        }
        let Some(vdd) = &self.vdd_net else {
            return;
        };
        let v = view.value(&format!("v({vdd})")).unwrap_or(0.0);
        let i_q = v / (3.3 / QUIESCENT_A);
        // Pin order follows `info.pins`: 3V3 first, then GND, then GPIOs.
        out[0] = -i_q;
        out[1] = i_q;
    }

    fn take_engine(&mut self) -> Option<Box<dyn Engine>> {
        // Analog OP is not available here; callers that need input levels should
        // use [`Self::build_engine`] after creating the analog engine.
        let boot = self.boot.take()?;
        let mut soc = Esp32c3::new();
        match boot {
            BootImage::Elf(bytes) => {
                let img = ElfImage::parse(&bytes).ok()?;
                soc.load_elf(&img).ok()?;
            }
        }
        let mut eng = Esp32c3Engine::new(&self.info.refdes, soc);
        for &pin in &self.gpios {
            eng.link_pin(PinLink {
                pin,
                target: self.analog_target,
            });
        }
        for &pad in &self.adc_pads {
            eng.link_adc(pad, self.analog_target);
        }
        for &pin in &self.input_pins {
            eng.link_input(pin, false);
        }
        Some(Box::new(eng))
    }
}
