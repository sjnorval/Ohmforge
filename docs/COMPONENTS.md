# Common components checklist

A living list of the parts a hobbyist or maker ESP32 project most often needs in Ohmforge. It is a planning aid for the parts library, not a schedule. The roadmap still targets "about 50 parts" in phase P3, each with a `part.toml`, behaviour and tests ([`docs/ROADMAP.md`](ROADMAP.md)).

Every Ohmforge part is a real, orderable part: data lives in `parts/<category>/…/part.toml` (CC-BY-SA-4.0), and behaviour is a Rust, WASM or SPICE model (AGPL-3.0-or-later). Request a part with the [New part or model](https://github.com/sjnorval/Ohmforge/issues/new?template=new_part.yml) issue form, or add one by following [CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-part).

Status: 20 November 2026 (Africa/Johannesburg), from branch `p2-adc-pwm` (`main` at `df7814e`, which includes GPIO inputs and the push button (#22), plus the ADC, LEDC and the potentiometer; not yet merged). Checkboxes and "already in the library" notes were taken from the tree, not from plans.

## How to read the list

- `[x]` means a `part.toml` for that part is already in `parts/`. A checked item can still be incomplete; the note says what the model actually covers.
- `[ ]` means there is no library part yet. The analog solver may already know the device class (for example diodes, inductors, MOSFETs); that is noted, but it is not the same as a placed, orderable part.
- **Modelling level:** analog SPICE-style, a digital or protocol model, or both (a mixed part: supply or power pins in the analog solver, bus pins in a digital model).
- **Interface:** the SoC or circuit hook the part needs (GPIO, ADC, PWM/LEDC, I2C, SPI, UART, 1-Wire, RMT, or passive/power only).
- **Priority:** checklist tiers, *not* the roadmap phases P0–P6.
  - **P1** essential: needed for a first useful board (passives, power, LED, button, potentiometer) and the MVP demo in the roadmap (blink, button interrupt, potentiometer to ADC, LED dimming).
  - **P2** common: typical maker projects once I2C, SPI, RMT and electromechanical models exist.
  - **P3** nice-to-have: useful, but narrower or blocked on later chips and peripherals.
- Unmodelled peripherals the part would depend on are named in the same line.

## Already in the library

Nine parts, all with an `[analog]` section. There is no digital or protocol model in `of-components` yet (`rust:bme280` is named in the BME280 `part.toml`, but only the supply load is implemented).

| Part id | Folder | What the model does |
|---|---|---|
| `generic.resistor` | `parts/passives/resistor` | Chip resistor, value per placement, tolerance and tempco |
| `generic.capacitor-mlcc` | `parts/passives/capacitor-mlcc` | X5R/X7R MLCC with ESR/ESL; DC-bias derating is not modelled |
| `generic.capacitor-tantalum` | `parts/passives/capacitor-tantalum` | 22 µF 16 V tantalum, including ESR for the AMS1117 output cap |
| `generic.led-red-0603` | `parts/leds/red-0603` | Red 0603 LED (Shockley plus light output), typical 2.0 V at 20 mA |
| `generic.npn-small-signal` | `parts/discretes/npn-small-signal` | MMBT3904-class Gummel-Poon NPN (Ohmforge parameters, not a vendor model) |
| `ams.ams1117-3.3` | `parts/regulators/ams1117` | AMS1117-3.3 1 A LDO macromodel (`ams1117.lib`), checked against the datasheet and ngspice |
| `bosch.bme280` | `parts/sensors/bosch-bme280` | GY-BME280 supply load only; I2C behaviour is not implemented |
| `generic.pushbutton` | `parts/switches/pushbutton-6mm` | 6 mm tactile push button (SPST-NO) as an ideal switch: 0.1 Ω closed, 100 MΩ open, pressed and released at run time; bounce is a sequence of presses and releases |
| `bourns.ptv09a-b103` | `parts/passives/bourns-ptv09a` | Bourns PTV09A 10 kΩ ±20 % linear potentiometer: two resistors split at the wiper (`position` 0 to 1), turned at run time |

These parts are already used in the blinky co-simulation (resistor + red LED), the esp-hal button co-simulation (push button on GPIO9 with the internal pull-up, LED on GPIO8; `crates/of-soc-esp32c3/tests/esp_hal_button.rs`), the esp-hal ADC/PWM co-simulation (potentiometer wiper on GPIO2, PWM-dimmed LED on GPIO8; `tests/esp_hal_adc_pwm.rs`) and in the MVP analog board check (USB 5 V stimulus, AMS1117, both capacitors, NPN switch and LEDs).

## Simulator readiness (what parts can hook up to)

Taken from `of-analog`, `of-components`, `of-soc-esp32c3` and the roadmap. This will go stale as P2 continues; treat the crates as the authority.

**Analog solver and parts bridge (phase P1, done).** The solver has R, C (ESR/ESL), L, coupled inductors, diodes (including LED and breakdown/Zener), Gummel-Poon BJTs, MOSFET levels 1 and 3, voltage/current sources, controlled sources, and voltage- or current-controlled switches (`S`/`W`). The parts bridge (`of_components::analog`) maps library parts onto these kinds: `resistor`, `capacitor`, `inductor`, `diode`, `led`, `npn`, `pnp`, `load`, `switch` (an ideal on/off resistor, `ron`/`roff`, switched at run time), `potentiometer` (two resistors split at the wiper, turned at run time) and `subckt`. There is not yet a parts-bridge kind for MOSFETs or for the solver's voltage- and current-controlled switches, so those devices can be used from a SPICE deck but not from a `part.toml` until a kind or a subcircuit is added. `of-digital` is still only a 4-state logic enum.

**ESP32-C3 peripherals already modelled, with gaps:**

- GPIO outputs: `GPIO_OUT` / `GPIO_ENABLE`, matrix `OUT_SEL` 128, inversion, IO MUX function and drive strength, a Thevenin pin model, and open-drain output (`PAD_DRIVER`).
- GPIO inputs (#22): `GPIO_IN` follows the circuit node through V_IH/V_IL threshold watches (0.75/0.25 VDD, acting as hysteresis), gated by `FUN_IE`; the weak pull-up and pull-down (45 kΩ) are part of the analog pin model; pin interrupts (edge and level) reach the CPU through the interrupt matrix; input reads happen at the exact access time, and a core in `wfi` wakes at the input event. The GPIO input-select matrix routes pins to peripheral input signals, but no modelled peripheral reads them yet; on the output side the matrix routes the LEDC channels to pins. Not modelled: input synchronisers and glitch filter, wake-up from sleep, leakage and pin capacitance. A busy core can take a pin interrupt up to one kernel slice late (boundable). Buttons and other digital inputs can now be built; protocols with tight bit timing still need their own models.
- UART0: TX capture, configuration registers and the configured baud rate; baud-timed RX FIFO (frame timing at the configured baud), RX interrupts (`RXFIFO_FULL`, `RXFIFO_TOUT`, `RXFIFO_OVF`) and a 64 KiB TX log cap. TX still lands instantly in the capture log.
- USB Serial/JTAG: TX byte capture only. USB itself is not modelled, and there is no data from a host.
- SPI0/SPI1: the flash memory controllers and a 4 MiB SPI NOR chip, used to boot firmware. This is not general-purpose SPI for sensors, displays or SD cards.
- ADC (branch `p2-adc-pwm`): SAR ADC one-shot conversions on ADC1 channels 0 to 4 (GPIO0 to GPIO4) and ADC2 channel 0 (GPIO5), four attenuations, sampling the circuit node at the conversion start. Codes follow Espressif's calibration curve with the chip's eFuse values, so calibrated readings (esp-hal `AdcCalCurve`) come back within 3 mV of the node voltage. Not modelled: noise and nonlinearity beyond the calibration curve, the input's sampling-capacitor load on the node, the ADC2 errata, continuous (DMA/pattern-table) mode, filters, thresholds and the temperature sensor.
- LEDC PWM (branch `p2-adc-pwm`): four timers and six channels (fractional dividers, 1- to 14-bit duty, Hpoint, dither, hardware fades, overflow counters, interrupts), each PWM edge at its exact counter tick in the circuit. The cost grows with PWM frequency (one kernel event per edge); there is no averaged mode.
- Also present: SYSTIMER, timer groups, watchdogs, the interrupt matrix, the SYSTEM CPU clock, RNG, and ROM/bootloader boot.

**Not modelled on the C3 (no register block in `of-soc-esp32c3` yet):** I2C, general-purpose SPI, RMT, TWAI/CAN, and UART RX. The roadmap places I2C, SPI, RMT and DAC on phase P3. The ESP32-C3 has no DAC; a DAC model is for classic ESP32 / S3. 1-Wire is not a C3 peripheral; it would be bit-banged GPIO (or RMT) once inputs and timing are honest.

**Other chips and the breadboard view.** `of-soc-esp32` is a placeholder. Classic ESP32 through a QEMU sidecar is phase P3; an owned Xtensa core is P4; ESP32-S3 and ESP32-C6 are P6. The breadboard view sits in P3 beside the schematic editor, which is itself still to do (the desktop app today is a demo picker for the blink, pot dimmer and LED fade).

---

## Passives

- [x] **Chip resistor** (`generic.resistor`) — Analog SPICE-style (R, tolerance, tempco); passive. P1. Already used as the LED series resistor; the same part covers pull-ups. No new peripheral needed.
- [x] **Ceramic chip capacitor, X5R/X7R MLCC** (`generic.capacitor-mlcc`) — Analog (C with ESR/ESL); passive. P1. DC-bias derating is documented as not modelled. No new peripheral needed.
- [x] **Tantalum chip capacitor** (`generic.capacitor-tantalum`) — Analog (22 µF 16 V with ESR); passive. P1. Present because the AMS1117 datasheet asks for it on the output. No new peripheral needed.
- [ ] **Aluminium electrolytic capacitor** — Analog (C with higher ESR/ESL and polarity); passive. P2. The capacitor kind already exists; this would be a new library part. No new peripheral needed.
- [ ] **Inductor or ferrite bead** — Analog (L with `rser`); passive. P2. The parts bridge already has an `inductor` kind, but there is no `part.toml`. No new peripheral needed.
- [ ] **Crystal or ceramic resonator** (for example 40 MHz, 32.768 kHz) — Analog (load capacitors plus a resonator model) or a board-level assumption; passive. P2. The C3 model already has an internal CPU clock (XTAL or PLL); a placed crystal is for completeness and for classic ESP32 boards, not for C3 blinky.

## Semiconductors

LEDs are listed under [Output](#output).

- [ ] **Small-signal diode** (1N4148 / SOD-123 class) — Analog (Shockley diode); passive. P1. The bridge already has a `diode` kind; no library part yet. No new peripheral needed.
- [ ] **Rectifier diode** (1N400x class) — Analog; passive. P2. Same `diode` kind, different ratings. No new peripheral needed.
- [ ] **Schottky diode** (SS14 / BAT54 class) — Analog; passive. P2. Common on power inputs and reverse-protection. No new peripheral needed.
- [ ] **Zener diode** (3.3 V and 5.1 V class) — Analog (diode with breakdown); passive. P2. The solver already models `BV`/`IBV`. No new peripheral needed.
- [x] **Small-signal NPN** (`generic.npn-small-signal`, MMBT3904 class) — Analog (Gummel-Poon); GPIO to the base through a resistor. P1. Already used as an LED/relay switch on the MVP board. Firmware hook is GPIO output, which is modelled.
- [ ] **Small-signal PNP** (MMBT3906 class) — Analog; GPIO. P2. The bridge already has a `pnp` kind; no library part yet. No new SoC peripheral needed.
- [ ] **N-channel MOSFET** (2N7002 / AO3400 class) — Analog; GPIO. P1. The solver has MOSFET levels 1 and 3, but the parts bridge has no `nmos`/`pmos` kind, so a library part needs a new kind or a subcircuit.
- [ ] **P-channel MOSFET** (high-side load switch, AO3401 class) — Analog; GPIO. P2. Same bridge gap as the N-channel part.
- [ ] **Power MOSFET** (logic-level, for motors and heavier loads) — Analog; GPIO or PWM/LEDC. P2. Needs the MOSFET parts kind. PWM drive can use LEDC.

## Power

- [ ] **USB 5 V source** — Analog (voltage source with some series resistance); power. P1. The MVP board test already injects a `VUSB` PWL 5 V stimulus in SPICE; it is not a library part. Planned as an energy part in roadmap phase P4. No SoC peripheral needed.
- [ ] **Li-ion / LiPo cell** (state of charge and internal resistance) — Analog; power. P2. Named in the roadmap for phase P4, with a power profiler. No SoC peripheral needed.
- [ ] **Coin cell** (CR2032 class) — Analog; power. P3. Also a phase P4 energy part.
- [x] **AMS1117-3.3 1 A LDO** (`ams.ams1117-3.3`) — Analog SPICE macromodel (`subckt`); power. P1. Dropout, quiescent current, line/load regulation, current limit, ripple rejection and a thermal network are modelled and checked against the datasheet and ngspice.
- [ ] **Other common 3.3 V LDOs** (for example AP2112, fixed 1117 variants) — Analog macromodel or `subckt`; power. P2. Same path as the AMS1117 part. No new peripheral needed.
- [ ] **Buck module** (MP1584 / LM2596 class) — Analog macromodel or functional voltage source plus ripple; power. P2. A full switching model is heavy; a datasheet-checked macromodel is enough for most firmware work. No new peripheral needed.
- [ ] **Boost module** (MT3608 class) — Analog macromodel; power. P3. Same modelling approach as the buck module.
- [ ] **Li-ion charger module** (TP4056 class) — Both (analog charge current and a simple charge-state model); power / GPIO status pins. P3. Depends on a battery model to be useful.

## Switches and input devices

- [x] **Tactile push-button** (`generic.pushbutton`) — Analog (ideal switch, `switch` kind) with the GPIO's internal or an external pull-up; GPIO. P1. Read by esp-hal firmware, polled and on a GPIO interrupt, with contact bounce driven as press/release events (`firmware/esp32c3-esp-hal-button`). Bounce is not generated by the part itself (the switch is still `ron`/`roff`); the live bench and the firmware tests share a seeded, toggleable script (`of_components::bounce`, default on, seed 0 is the canonical train). Contact resistance does not vary.
- [ ] **Slide switch** — Analog switch; GPIO. P2. Can reuse the `switch` kind (SPDT needs two switches or a `subckt`); GPIO inputs now exist.
- [ ] **DIP switch** — Analog switches; GPIO. P3. One `switch` per position; GPIO inputs now exist.
- [x] **Potentiometer** (`bourns.ptv09a-b103`, 10 kΩ linear, 9 mm) — Analog (`potentiometer` kind: two resistors split at the wiper; placement `position`, turned at run time with `BuiltBoard::pot_params`); ADC. P1. Read through the C3 ADC by esp-hal firmware that dims an LED (`firmware/esp32c3-esp-hal-adc-pwm`). Linear taper only; end (residual) resistance, wiper noise and the taper's shape error are not modelled. The desktop dimmer demo turns it at run time with a slider.
- [ ] **Rotary encoder** (incremental, with or without a switch) — Digital quadrature on two GPIOs, plus analog contact bounce if desired; GPIO. P2. GPIO inputs and edge interrupts now exist; needs a quadrature model that turns rotation into contact events.
- [ ] **Matrix keypad** (4×4 class) — Digital scan (row drive, column sense) with analog optionally for bounce and ghosting; GPIO. P3. GPIO inputs now exist; 16 `switch` devices in a matrix would model it.

## Output

- [x] **Red 0603 LED** (`generic.led-red-0603`) — Analog (LED Shockley model with `popt()` / `bright()` probes); GPIO. P1. Already co-simulated with a 330 Ω series resistor on GPIO8 in the blinky tests and the desktop demo. Dimmed and faded through LEDC PWM by esp-hal firmware (`firmware/esp32c3-esp-hal-adc-pwm`).
- [ ] **Other LED colours and sizes** (green, yellow, blue, white; 3 mm / 5 mm through-hole) — Analog, same `led` kind with different Vf and wavelength; GPIO. P2. No new peripheral needed for on/off; PWM dimming can use LEDC.
- [ ] **Discrete RGB LED** (common cathode or anode) — Analog, three LED devices; GPIO or PWM/LEDC. P2. Full colour mixing can use three LEDC channels.
- [ ] **Addressable RGB LED, WS2812 / NeoPixel class** — Digital protocol model (and analog supply current if possible); RMT (or very tight GPIO timing). P2. Blocked on RMT, which is a phase P3 peripheral. A bit-banged GPIO path is not realistic until output timing is much tighter than today.
- [ ] **7-segment display** (single or four-digit, raw or TM1637 class) — Digital (segment map) plus analog LED loads; GPIO or I2C, sometimes a protocol chip. P2. A raw display needs GPIO outputs (modelled) and usually a driver or many pins; TM1637 needs a two-wire protocol on GPIO. An I2C backpack needs I2C (not modelled).
- [ ] **Buzzer** (active on/off, or passive driven by a square wave) — Analog (simple load, optional sound probe); GPIO or PWM/LEDC. P2. An active buzzer can already be driven from a GPIO output. A passive buzzer can be driven by LEDC (or timed GPIO); it still needs a part.
- [ ] **Relay** (coil and contacts) — Analog (coil inductance and contact switch) plus GPIO drive, often through the NPN already in the library; GPIO. P2. Named in the roadmap under phase P3 electromechanical parts. The parts bridge has no switch kind yet; contacts can use a `subckt` or a new kind on top of the solver's `S`/`W` switches. Firmware only needs GPIO output.

## Motors and drivers

Electromechanical models (DC motor with back-EMF and inertia, servo) are listed on the roadmap for phase P3.

- [ ] **DC motor** — Analog (back-EMF, inertia, stall current); PWM/LEDC or GPIO through a driver. P2. Blocked on an electromechanical model; LEDC can provide speed-control PWM.
- [ ] **Hobby servo** (SG90 class) — Both (pulse-width protocol plus an analog or kinematic shaft); PWM/LEDC (50 Hz, ~1–2 ms pulse). P2. LEDC can produce the pulses (14-bit at 50 Hz); blocked on the servo model.
- [ ] **Stepper motor** (28BYJ-48 or NEMA 17 class) — Both (phase currents and steps); GPIO or a step/dir driver. P3. Blocked on a stepper model; GPIO outputs exist for direct-drive coils through a driver.
- [ ] **H-bridge, L298N class** — Analog (or a mixed functional model of the two half-bridges); GPIO / PWM/LEDC. P2. Needs the motor model to be interesting; LEDC can drive the PWM enable inputs.
- [ ] **H-bridge, DRV8833 class** — Analog or mixed; GPIO / PWM/LEDC. P2. Same dependencies as the L298N, at lower voltage and current.
- [ ] **Stepper driver, A4988 / DRV8825 class** — Digital step/dir/enable plus analog coil current; GPIO (and PWM if used). P3. Fault/home pins can use GPIO inputs; blocked on a stepper model. Step pulses can use GPIO outputs once timing is good enough; a dedicated PWM/LEDC path would help.

## Sensors

- [ ] **DHT11** — Digital protocol (single-wire, not 1-Wire) plus a supply load; GPIO. P2. GPIO inputs (open drain with pull-up) now exist; needs a protocol model with honest bit timing so the start pulse and the data bits can be sampled.
- [ ] **DHT22 / AM2302** — Digital protocol, same interface as the DHT11; GPIO. P2. Same protocol-model and timing dependency.
- [ ] **DS18B20** — Digital protocol (1-Wire) plus a supply load; 1-Wire on GPIO, sometimes parasitic power. P2. Open drain, pulls and GPIO inputs now exist; needs a 1-Wire device model with its slot timing. There is no 1-Wire peripheral to wait for; this is bit-banged.
- [x] **BME280** (`bosch.bme280`, GY-BME280 breakout) — Both intended; only the analog supply load exists today; I2C. P2. The `part.toml` lists I2C addresses 0x76/0x77 and names `rust:bme280`, but `of-components` has no I2C model, and the C3 I2C peripheral is not modelled (phase P3).
- [ ] **MPU6050** (or a later MPU/ICM 6-axis IMU) — Digital protocol plus a supply load; I2C (sometimes SPI). P2. Blocked on I2C (and SPI if that path is offered).
- [ ] **HC-SR04 ultrasonic** — Digital (trigger pulse, echo pulse width) with optional analog supply current; GPIO. P2. GPIO inputs now exist; needs the sensor's echo model. Measuring the pulse width needs only GPIO plus a timer.
- [ ] **PIR motion** (HC-SR501 class) — Digital level, optional analog supply; GPIO. P2. GPIO inputs now exist; needs a part with a scriptable output.
- [ ] **LDR (photoresistor)** — Analog (light-dependent resistance in a divider); ADC. P2. The C3 ADC now exists; needs a light-dependent resistance model.
- [ ] **NTC thermistor** — Analog (resistance against temperature, typically a 10 kΩ divider); ADC. P2. The C3 ADC now exists; needs a temperature-dependent resistance part. A simple interpolating table is enough; a full Steinhart–Hart model can come later.
- [ ] **Hall sensor** (digital latch such as A3144, or a linear hall) — Digital GPIO, or analog into ADC. P2. The digital form can use GPIO inputs; the linear form can use the ADC once a part exists.
- [ ] **Soil-moisture probe** (resistive or capacitive module) — Analog (or a digital module output); ADC or GPIO. P3. A resistive probe is an analog load into the ADC. A digital comparator module only needs GPIO inputs, which now exist.
- [ ] **INA219 current/voltage sensor** — Both (analog shunt and bus sense, I2C register map); I2C. P2. A useful first model is digital-only (scripted current/voltage). A mixed model would also stamp a shunt in the analog circuit. Blocked on I2C.

## Displays

- [ ] **SSD1306 OLED** (128×64, I2C; SPI variants exist) — Digital protocol (frame buffer); I2C or SPI. P2. Named as a built-in part in the design plan's crate map, but there is no library part or model yet. Blocked on I2C, or on general-purpose SPI for the SPI modules.
- [ ] **16×2 HD44780 character LCD with an I2C backpack** (PCF8574 class) — Digital protocol; I2C (raw HD44780 would be many GPIOs). P2. Blocked on I2C. A raw parallel model could use GPIO outputs and can read the busy flag through GPIO inputs.
- [ ] **ST7735 TFT** — Digital protocol (pixel writes); SPI, plus GPIO for DC/RST/CS. P3. Blocked on general-purpose SPI (SPI0/SPI1 are flash only).
- [ ] **ILI9341 TFT** — Digital protocol; SPI (sometimes 8080 parallel), plus GPIO. P3. Same SPI dependency as the ST7735.
- [ ] **e-paper module** (SSD1680 / Waveshare class) — Digital protocol; SPI plus GPIO (DC, RST, busy). P3. Blocked on general-purpose SPI (the busy pin can use GPIO inputs).

## Communication modules

- [ ] **USB-UART bridge** (CP2102, CH340 or CH343 class) — Digital (UART data path); UART. P1. Needed on classic DevKit-style boards. The C3 often uses its own USB Serial/JTAG, whose TX path is already captured. A discrete bridge part still needs UART RX to send data into firmware, and UART baud timing is not modelled. USB itself is not modelled.
- [ ] **Logic level shifter** (TXS0108E or BSS138 bidirectional class) — Analog (or a mixed voltage-domain model); often I2C or UART or SPI on both sides. P2. A first model can be analog (FETs and pull-ups) so missing 3.3/5 V shifting shows up on the scope. Open-drain I2C shifting also needs I2C to be useful in a real bus.
- [ ] **RS-485 transceiver** (MAX485 class) — Both (analog bus plus digital DE/RE); UART and GPIO. P3. Blocked on UART RX (and baud timing). TWAI is not involved.
- [ ] **CAN transceiver** (SN65HVD230 class) — Both (analog bus plus digital TX/RX); the C3 TWAI controller. P3. Blocked on TWAI, which is not modelled (and is not on the P3 peripheral list, which names I2C, SPI, RMT and DAC).
- [ ] **SD card** (SPI mode, microSD slot or breakout) — Digital protocol (and optional analog supply current); SPI, plus a CS GPIO. P2. Blocked on general-purpose SPI. This is not the on-chip flash behind SPI0/SPI1.

## ICs

- [ ] **555 timer** (NE555 / TLC555) — Analog macromodel or `subckt`; passive / GPIO. P3. Can be built from solver primitives or a subcircuit; no new SoC peripheral needed for a free-running oscillator. A 555 output can drive a GPIO input.
- [ ] **Op-amp** (LM358 class) — Analog macromodel or `subckt`; analog pins, sometimes ADC. P2. Named with AMS1117 and TL431 in the roadmap for phase P3. The `subckt` path already works (as the AMS1117 shows). Firmware can read the result through the ADC.
- [ ] **Comparator** (LM393 class) — Analog macromodel; GPIO or analog. P2. Same `subckt` path. Its output can drive a C3 GPIO input.
- [ ] **Shunt reference, TL431** — Analog macromodel or `subckt`; power / analog. P2. Also named in the phase P3 macromodel work. No SoC peripheral needed.
- [ ] **Logic gates** (74HC00 / 08 / 32 / 04 class) — Digital (and analog I/O levels if a mixed model is used); GPIO. P3. `of-digital` is only a 4-state logic stub, so even a purely digital gate needs that kernel to grow. Electrical levels would use the analog solver.
- [ ] **Shift register, 74HC595** — Digital (serial-in, parallel-out); GPIO or SPI. P2. Can be bit-banged on GPIO outputs (modelled) once a digital model exists; a SPI-attached version needs general-purpose SPI. Cascade and output-enable are GPIO.
- [ ] **Shift register, 74HC165** — Digital (parallel-in, serial-out); GPIO or SPI. P2. Blocked on a digital model (GPIO inputs exist).
- [ ] **I2C GPIO expander** (PCF8574 or MCP23017 class) — Digital protocol; I2C. P2. Blocked on I2C. The PCF8574 is also the usual HD44780 backpack.
- [ ] **External ADC** (ADS1115 class) — Digital protocol, optional analog input pins; I2C. P3. A digital-only first model can feed scripted samples. A mixed model would sample analog nets. Blocked on I2C. Useful for higher resolution than the C3 ADC.
- [ ] **External DAC** (MCP4725 class) — Digital protocol plus an analog output voltage; I2C. P3. The C3 has no DAC; this is the usual hobby substitute. Blocked on I2C, and on writing that voltage into the analog circuit.

## Boards and interconnect

There are no board, breadboard, wire or connector parts in `parts/` today. The ESP32-C3 *SoC* is modelled in `of-soc-esp32c3`; that is not the same as a DevKit part with a pinout, USB, regulator and onboard LED.

- [ ] **ESP32-C3 DevKitM-1 class board** — Both (board pinout, USB, regulator, strapping, onboard LED/button, plus the existing C3 SoC); GPIO, UART or USB Serial/JTAG, power. P1. The SoC, ROM boot, flash and GPIO output path already exist. A board part still needs a pin map, usually the AMS1117 (already in the library) or another 3.3 V regulator, USB 5 V, and the onboard button (the push button and GPIO inputs now exist; GPIO9 is the BOOT button, also a strapping pin). Some C3 boards use the SoC's USB Serial/JTAG; others include a discrete USB-UART chip.
- [ ] **ESP32-C3 SuperMini class board** — Both, same pieces as the DevKitM in a smaller pinout; GPIO, USB Serial/JTAG, power. P1. Same SoC and the same missing board-level part.
- [ ] **ESP32 DevKitC class board** (classic Xtensa ESP32) — Both; GPIO, UART (usually a CP2102 or CH340), ADC, LEDC, and later I2C/SPI/RMT. P2. `of-soc-esp32` is a placeholder crate. Classic ESP32 is planned through a QEMU sidecar in phase P3, then an owned Xtensa core in P4.
- [ ] **ESP32-S3 DevKit class board** — Both; GPIO, USB, ADC, LEDC, I2C/SPI/RMT. P3. ESP32-S3 support is planned in phase P6; there is no SoC model yet.
- [ ] **Solderless breadboard** (830-point / 400-point class) — Connectivity and mechanical fit, not a SPICE device; interconnect. P1. The part is the place to build; the roadmap's breadboard view (true 2.54 mm geometry, shorted rows, parts that do not fit) is phase P3 and depends on the schematic editor, which is not built yet.
- [ ] **Jumper wires** (male–male, male–female) — BOM and breadboard artwork, not an electrical model beyond the net they belong to; interconnect. P1. Useful once a breadboard or schematic view can show wire colour and length (phase P3 view).
- [ ] **Pin headers and Dupont connectors** — BOM and footprint; interconnect. P1. No SoC peripheral needed.
- [ ] **USB connector** (USB-C or Micro-USB) — BOM, and a hook for the USB 5 V source; power / USB. P2. USB data is not modelled; USB Serial/JTAG on the C3 is TX-only capture.
- [ ] **JST-PH battery connector** (2-pin, as used on LiPo packs and TP4056 modules) — BOM plus connection to a battery part; power. P2. Depends on a battery part to matter.
- [ ] **Screw terminals** — BOM and footprint; interconnect. P3. No SoC peripheral needed.

---

## Counts

Checklist tiers below are the P1/P2/P3 *component* priorities, not roadmap phases. "In library" counts `part.toml` entries, including the partial BME280.

| Category | Items | In library | P1 | P2 | P3 |
|---|---:|---:|---:|---:|---:|
| Passives | 6 | 3 | 3 | 3 | 0 |
| Semiconductors | 9 | 1 | 3 | 6 | 0 |
| Power | 8 | 1 | 2 | 3 | 3 |
| Switches and input devices | 6 | 2 | 2 | 2 | 2 |
| Output | 7 | 1 | 1 | 6 | 0 |
| Motors and drivers | 6 | 0 | 0 | 4 | 2 |
| Sensors | 12 | 1 | 0 | 11 | 1 |
| Displays | 5 | 0 | 0 | 2 | 3 |
| Communication modules | 5 | 0 | 1 | 2 | 2 |
| ICs | 10 | 0 | 0 | 6 | 4 |
| Boards and interconnect | 10 | 0 | 5 | 3 | 2 |
| **Total** | **84** | **9** | **17** | **48** | **19** |

The nine library parts are: the chip resistor, MLCC, tantalum capacitor, red 0603 LED, small-signal NPN, AMS1117-3.3, the BME280 supply load, the tactile push button and the potentiometer.

## Suggested order

This is not a commitment, just a way to grow the library in step with the SoC:

1. **While GPIO output and the analog solver already work:** the missing passives and discretes that the bridge can already express (signal diode, inductor, PNP), then an N-channel MOSFET kind, a USB 5 V source part, and extra LED colours. These do not wait on new peripherals.
2. **With the P2 MVP peripherals (GPIO inputs, ADC, LEDC):** button and potentiometer (done), more analog sensors (LDR, thermistor), LED dimming on the existing LED, a buzzer, and a first DevKit-style C3 board part (pinout, regulator, USB 5 V, onboard LED and button).
3. **With phase P3 buses and electromechanical models (I2C, SPI, RMT, plus motor/relay/servo):** finish the BME280, then SSD1306, the HD44780 backpack, INA219, MPU6050, DS18B20, DHT11/22, WS2812, SD card, level shifters, servo, DC motor and the common H-bridges. That set is most of the "common hobby projects run" exit for 0.2.
4. **Later:** classic ESP32 and S3 boards, energy parts (LiPo, coin cell), TFTs and e-paper, RS-485 and CAN, steppers, and the remaining ICs.

## See also

- [`parts/README.md`](../parts/README.md) — current library and the folder layout
- [`parts/LICENSE.md`](../parts/LICENSE.md) — CC-BY-SA-4.0 rules; no vendor SPICE models or datasheets in the tree
- [`docs/ROADMAP.md`](ROADMAP.md) — phase checklists, including the 50-part and electromechanical items
- [`docs/DESIGN_AND_PLAN.md`](DESIGN_AND_PLAN.md) — device-model table (section 4) and the part format
- [CONTRIBUTING.md](../CONTRIBUTING.md#adding-a-part) — how to add a part
- [New part or model issue form](https://github.com/sjnorval/Ohmforge/issues/new?template=new_part.yml)
