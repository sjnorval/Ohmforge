//! ESP32-C3 as an engine [`Component`] (RFC 0001 / design §3c).

use std::collections::BTreeMap;

use of_analog::devices::Resistor;
use of_analog::{AnalogEngine, Edge};
use of_component_api::{AttachCtx, Component, ComponentInfo, ElabCtx, ElabError, PinKind, PinSpec, SolutionView};
use of_core::{Engine, SimTime};
use of_engine::{ComponentSpec, Firmware, Registry};
use of_firmware::ElfImage;

use crate::engine::{Esp32c3Engine, PinLink, adc_request_code, adc_value_code, initial_input_level, input_code};
use crate::gpio::{GPIO_COUNT, PinOut, VIH_FRACTION, VIL_FRACTION};
use crate::pad::{add_pin_drivers, hook_pin};
use crate::rom::{RomImage, boot_from_rom};
use crate::soc::Esp32c3;

/// Register `rust:esp32c3` ([`Esp32c3Component::from_spec`]).
pub fn register(reg: &mut Registry) {
    reg.register(
        "rust:esp32c3",
        Box::new(|spec: ComponentSpec<'_>| Ok(Box::new(Esp32c3Component::from_spec(spec)?) as Box<dyn Component>)),
    );
}

/// Quiescent current estimate drawn from 3V3 (A); datasheet modem-sleep order.
pub const QUIESCENT_A: f64 = 5e-3;

/// Port name for UART0 (TX/RX) that a UART console instrument can attach to.
pub const UART0_PORT: &str = "uart0";

/// How the component gets a running chip. The image is loaded (or the ROM
/// boot is run) when the component is built, so the pads start in the state
/// the firmware left them in.
pub enum BootImage {
    /// Direct ELF load ([`Esp32c3::load_elf`]).
    Elf(Vec<u8>),
    /// Mask ROM, espflash's second-stage bootloader, then the app from flash
    /// ([`crate::rom::boot_from_rom`]), until UART0 shows `ready_text`.
    Rom {
        /// Application image (`.bin`).
        app: Vec<u8>,
        /// Partition table.
        partitions: Vec<u8>,
        /// Mask ROM ELF bytes.
        rom: Vec<u8>,
        /// Second-stage bootloader ELF bytes.
        bootloader: Vec<u8>,
        /// UART text that marks the end of boot.
        ready_text: String,
    },
    /// An already booted chip (for example the desktop pre-boot cache). The
    /// circuit connects at the chip's current time ([`Self::origin`] of the
    /// session).
    Booted(Box<Esp32c3>),
}

impl std::fmt::Debug for BootImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Elf(b) => write!(f, "Elf({} bytes)", b.len()),
            Self::Rom { ready_text, .. } => write!(f, "Rom {{ ready_text: {ready_text:?} }}"),
            Self::Booted(_) => f.write_str("Booted(..)"),
        }
    }
}

impl BootImage {
    fn into_soc(self) -> Result<Esp32c3, ElabError> {
        match self {
            Self::Elf(bytes) => {
                let img = ElfImage::parse(&bytes).map_err(|e| ElabError::msg(e.to_string()))?;
                let mut soc = Esp32c3::new();
                soc.load_elf(&img).map_err(|e| ElabError::msg(e.to_string()))?;
                Ok(soc)
            }
            Self::Rom {
                app,
                partitions,
                rom,
                bootloader,
                ready_text,
            } => {
                let elf = ElfImage::parse(&rom).map_err(|e| ElabError::msg(format!("ROM ELF did not parse: {e}")))?;
                let rom = RomImage::from_elf(&elf).map_err(|e| ElabError::msg(format!("ROM image: {e}")))?;
                boot_from_rom(&rom, &bootloader, &app, &partitions, &ready_text).map_err(ElabError::msg)
            }
            Self::Booted(soc) => Ok(*soc),
        }
    }
}

/// ESP32-C3 chip component: supply pins, selected GPIOs, and an active CPU engine.
pub struct Esp32c3Component {
    info: ComponentInfo,
    /// The loaded or booted chip until [`Component::take_engine`] moves it
    /// into the CPU engine.
    soc: Option<Esp32c3>,
    /// GPIO numbers elaborated (from connected `GPIOn` pins).
    gpios: Vec<u8>,
    /// Pads that should take their level from the analog circuit (inputs).
    input_pins: Vec<u8>,
    /// Initial levels of the input pads, from the operating point (attach).
    input_levels: BTreeMap<u8, bool>,
    /// ADC pads among connected GPIOs (0–5).
    adc_pads: Vec<u8>,
    /// Kernel event target for pin/ADC events (set before [`Self::take_engine`]).
    analog_target: u32,
    /// Net name for 3V3 after elaborate.
    vdd_net: Option<String>,
    /// GPIO number → circuit net name, for ADC `sample_on` and input watches.
    gpio_nets: BTreeMap<u8, String>,
    /// Supply current drawn while running (A); 0 adds no load device.
    quiescent_a: f64,
}

fn pin_specs(refdes: &str, pins: &[&str]) -> Result<Vec<PinSpec>, ElabError> {
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
                let n = parse_gpio(other).ok_or_else(|| ElabError::msg(format!("{refdes}: unknown pin '{other}'")))?;
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
    // Supply pins first so `terminal_currents` can rely on the order.
    specs.sort_by_key(|p| match p.kind {
        PinKind::Power => 0,
        PinKind::Ground => 1,
        _ => 2,
    });
    Ok(specs)
}

impl Esp32c3Component {
    /// Place the chip as `refdes` with a direct-ELF image and the listed pins.
    ///
    /// `pins` must include `3V3` and `GND`, plus each `GPIOn` that is wired
    /// (only connected pins are declared — design §3c).
    pub fn with_elf(refdes: &str, elf: &[u8], pins: &[&str]) -> Result<Self, ElabError> {
        Self::with_boot(refdes, BootImage::Elf(elf.to_vec()), pins)
    }

    /// Place the chip as `refdes`, loading or booting `boot` now.
    pub fn with_boot(refdes: &str, boot: BootImage, pins: &[&str]) -> Result<Self, ElabError> {
        let specs = pin_specs(refdes, pins)?;
        let soc = boot.into_soc().map_err(|e| ElabError::msg(format!("{refdes}: {e}")))?;
        Ok(Self {
            info: ComponentInfo {
                part_id: "espressif.esp32-c3".into(),
                refdes: refdes.to_owned(),
                pins: specs,
            },
            soc: Some(soc),
            gpios: Vec::new(),
            input_pins: Vec::new(),
            input_levels: BTreeMap::new(),
            adc_pads: Vec::new(),
            analog_target: 1,
            vdd_net: None,
            gpio_nets: BTreeMap::new(),
            quiescent_a: QUIESCENT_A,
        })
    }

    /// Build from a registry spec (model `rust:esp32c3`, E4b).
    ///
    /// Params: `quiescent_a` (A, default [`QUIESCENT_A`]; 0 adds no load),
    /// `inputs` (a list such as `["GPIO9"]`; each must be a connected pin) and
    /// `cpu_mhz` (reserved; must be 160).
    pub fn from_spec(mut spec: ComponentSpec<'_>) -> Result<Self, ElabError> {
        let refdes = spec.refdes;
        let boot = match spec.firmware.take() {
            None => return Err(ElabError::msg(format!("{refdes}: no firmware for the ESP32-C3"))),
            Some(Firmware::Elf(bytes)) => BootImage::Elf(bytes),
            Some(Firmware::Rom {
                app,
                partitions,
                rom,
                bootloader,
                ready_text,
            }) => BootImage::Rom {
                app,
                partitions,
                rom,
                bootloader,
                ready_text: ready_text
                    .ok_or_else(|| ElabError::msg(format!("{refdes}: a ROM boot needs a ready text")))?,
            },
            Some(Firmware::Booted(any)) => BootImage::Booted(
                any.downcast::<Esp32c3>()
                    .map_err(|_| ElabError::msg(format!("{refdes}: pre-booted firmware is not an ESP32-C3")))?,
            ),
        };
        if let Some(v) = spec.param("cpu_mhz") {
            let mhz = v
                .as_f64()
                .ok_or_else(|| ElabError::msg(format!("{refdes}: cpu_mhz must be a number")))?;
            if mhz != 160.0 {
                return Err(ElabError::msg(format!(
                    "{refdes}: cpu_mhz = {mhz} is not supported (only 160)"
                )));
            }
        }
        let quiescent_a = match spec.param("quiescent_a") {
            None => QUIESCENT_A,
            Some(v) => v
                .as_f64()
                .filter(|a| a.is_finite() && *a >= 0.0)
                .ok_or_else(|| ElabError::msg(format!("{refdes}: quiescent_a must be a number >= 0")))?,
        };
        let inputs: Vec<String> = match spec.param("inputs") {
            None => Vec::new(),
            Some(v) => v
                .as_str_list()
                .ok_or_else(|| ElabError::msg(format!("{refdes}: inputs must be a list of pin names")))?
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };
        let pins: Vec<&str> = spec.pins.iter().map(String::as_str).collect();
        let mut chip = Self::with_boot(refdes, boot, &pins)?;
        chip.quiescent_a = quiescent_a;
        for name in &inputs {
            let n = parse_gpio(name)
                .filter(|n| chip.info.pins.iter().any(|p| parse_gpio(&p.name) == Some(*n)))
                .ok_or_else(|| ElabError::msg(format!("{refdes}: input '{name}' is not a connected GPIO pin")))?;
            chip.enable_input(n);
        }
        Ok(chip)
    }

    /// Supply current drawn while running (A).
    pub fn quiescent_a(&self) -> f64 {
        self.quiescent_a
    }

    /// Set the supply current drawn while running (A); 0 adds no load device.
    pub fn set_quiescent_a(&mut self, amps: f64) {
        self.quiescent_a = amps.max(0.0);
    }

    /// The chip before it moves into its engine (UART, pins, time).
    pub fn soc(&self) -> Option<&Esp32c3> {
        self.soc.as_ref()
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
    ///
    /// Thin wrapper kept for one release: [`Component::attach`] then
    /// [`Component::take_engine`] does the same through `Session`.
    pub fn build_engine(&mut self, analog: &AnalogEngine) -> Result<Esp32c3Engine, ElabError> {
        self.record_input_levels(|name| analog.value(name));
        self.make_engine()
            .ok_or_else(|| ElabError::msg("Esp32c3Component: firmware already consumed"))
    }

    fn record_input_levels(&mut self, value: impl Fn(&str) -> Option<f64>) {
        let Some(vdd_net) = &self.vdd_net else {
            return;
        };
        let vdd = value(&format!("v({vdd_net})")).unwrap_or(3.3);
        for &pin in &self.input_pins {
            let net = self.gpio_nets.get(&pin).map(String::as_str).unwrap_or("gnd");
            let v = value(&format!("v({net})")).unwrap_or(0.0);
            self.input_levels.insert(pin, initial_input_level(v, vdd.max(0.1)));
        }
    }

    fn make_engine(&mut self) -> Option<Esp32c3Engine> {
        let soc = self.soc.take()?;
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
            eng.link_input(pin, self.input_levels.get(&pin).copied().unwrap_or(false));
        }
        Some(eng)
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
            let initial = self.soc.as_ref().map_or(PinOut::HIZ, |soc| soc.pin(usize::from(num)));
            add_pin_drivers(cx.circuit_mut(), &self.info.refdes, num, gpio, vdd, gnd, initial)
                .map_err(ElabError::msg)?;
            self.gpios.push(num);
            self.gpio_nets.insert(num, net);
            if num <= 5 {
                self.adc_pads.push(num);
            }
        }

        if self.quiescent_a > 0.0 {
            let r_iq = 3.3 / self.quiescent_a;
            let iq = format!("{}_iq", self.info.refdes.to_ascii_lowercase());
            cx.add_device(Box::new(Resistor::new(&iq, vdd, gnd, r_iq)))?;
        }
        Ok(())
    }

    fn attach(&mut self, cx: &mut dyn AttachCtx) -> Result<(), ElabError> {
        self.analog_target = cx.analog_id();
        let cpu = cx
            .own_engine_id()
            .ok_or_else(|| ElabError::msg(format!("{}: no engine id for the CPU", self.info.refdes)))?;
        self.hook_gpio_outputs(cx.analog());
        self.hook_gpio_inputs(cx.analog(), cpu).map_err(ElabError::msg)?;
        self.hook_adc(cx.analog()).map_err(ElabError::msg)?;
        self.record_input_levels(|name| cx.op_value(name));
        if let Some(soc) = &self.soc {
            let t = soc.hart.local_time();
            if t > SimTime::ZERO {
                cx.set_origin(t);
            }
        }
        Ok(())
    }

    fn has_engine(&self) -> bool {
        self.soc.is_some()
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
        let i_q = v * self.quiescent_a / 3.3;
        // Pin order follows `info.pins`: 3V3 first, then GND, then GPIOs.
        out[0] = -i_q;
        out[1] = i_q;
    }

    fn take_engine(&mut self) -> Option<Box<dyn Engine>> {
        // Input levels come from `attach` (or `build_engine`); without them
        // inputs start low.
        self.make_engine().map(|e| Box::new(e) as Box<dyn Engine>)
    }
}
