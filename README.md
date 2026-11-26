<p align="center">
  <img src="assets/social-preview.png" alt="Ohmforge — emulate ESP32 in the browser. No board required." width="700">
</p>

<p align="center">
  <a href="https://github.com/sjnorval/Ohmforge/actions/workflows/ci.yml"><img src="https://github.com/sjnorval/Ohmforge/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-AGPL--3.0--or--later-blue" alt="Licence: AGPL-3.0-or-later"></a>
</p>

An open-source, true-to-life ESP32 circuit simulator written in Rust, with built-in bill-of-materials tracking.

**Status:** pre-alpha. The SPICE-grade analog solver (`of-analog`, checked against ngspice), the RV32IMC CPU core (`of-cpu-riscv`, passing riscv-tests), the co-simulation kernel (`of-core`) and the parts-to-devices bridge (`of-components`) work. The ESP32-C3 SoC (`of-soc-esp32c3`) boots firmware from an ELF or an ESP flash image: a blinky drives a simulated LED and resistor end to end, and timer interrupts and watchdogs behave as on the chip. Espressif's real boot ROM runs on the model and boots firmware from simulated SPI flash through the cache MMU; an unmodified esp-hal blinky boots through the ROM and the ESP-IDF second-stage bootloader and drives the LED, esp-hal firmware reads a simulated push button (polled or on a GPIO interrupt) through realistic input thresholds and pull-ups, and it reads a potentiometer through the calibrated ADC and dims and fades an LED with LEDC PWM, every PWM edge reaching the circuit at its exact time. In the desktop demo, the LED lights on the schematic in sync with the simulated current waveform. The app also shows the firmware's UART0 output, a configurable bench supply on the VDD rail (its − terminal routed to GND, with period-averaged current on the face and sidebar), and click-to-place virtual voltmeters and ammeters. The plan is in [`docs/DESIGN_AND_PLAN.md`](docs/DESIGN_AND_PLAN.md); progress in [`docs/ROADMAP.md`](docs/ROADMAP.md); the parts we plan to support in [`docs/COMPONENTS.md`](docs/COMPONENTS.md).

## Goals

- Run unmodified ESP32 firmware (ESP-IDF, Arduino-ESP32, esp-hal, MicroPython) against a SPICE-grade analog circuit, in one deterministic simulation. MicroPython on the ESP32-C3 is scheduled in P4 ([`docs/design/micropython.md`](docs/design/micropython.md)).
- Native desktop app and a WASM web app from one codebase. Firmware is written in your editor, not inside Ohmforge ([`docs/design/editor-integration.md`](docs/design/editor-integration.md)).
- Every part is a real, orderable part, so the BOM, cost (ZAR) and build checklist come straight from the design.

## Planned changes: base engine and plug-in components

> **Architecture in progress.** The design is merged (phase E1, #43). The bench PSU already has explicit `+` and `−` terminals, with `−` routed to GND on the demo boards, and the LED and PSU currents are averaged over whole PWM periods (#45, closed by #54). Ohmforge is moving to a **base engine** that stands in for the real world: it owns the nets, the analog and digital solvers, simulated time and events, and the wiring, readings, logging and measuring services. **Every part with a real-world counterpart becomes a plug-in component** on one engine API: resistors, LEDs, pots and buttons, the **bench PSU** and the **ESP32-C3 itself**. Meters, the scope, the logger and the UART console become instruments that work on any circuit; demos become circuit files drawn by the existing schematic router; and the BOM comes straight from the components. Engine phases E2 and E3 are on `main` (#62, #63), and E4a, the circuit file format, landed in #73; E4b-1, the engine's model registry and `Session` runtime, landed in #77, and E4b-2, the demos as circuit files in `circuits/demos/` with `ohmforge run <file.circuit.toml>`, landed in #78. E4c, the desktop app drawing and running the demos from those files, landed in #79; with E4d the files are the desktop's only path and the hand-wired demos are gone, which completes E4. The rest of E4, and E5 and E6, follow in P3 / 0.2 (#48 to #50); 0.1 does not wait for them. The design is in [`docs/design/engine-components.md`](docs/design/engine-components.md).

**Planned: editor integration.** Upload, serial monitor and debugging from VS Code and the Arduino IDE 2.x as if the simulator were a real board: a local control API, a virtual serial port, a GDB stub and hot reload in P3, then a VS Code extension and an Arduino board package in P4. The plan is in [`docs/design/editor-integration.md`](docs/design/editor-integration.md).

**Planned: Arduino-ESP32 and ESP-IDF MVP demos.** The four MVP behaviours (blink, button, pot dimmer, LED fade) already run from esp-hal; #60 adds the same pin-parity apps as stock Arduino-ESP32 and ESP-IDF firmware, booted through the real ROM and bootloader, with committed flash images and CI rebuilds. That is separate from the Arduino IDE board package (I6 in P4). The plan is in [`docs/design/arduino-idf-mvp-demos.md`](docs/design/arduino-idf-mvp-demos.md).

**Planned: parts availability.** Keep BOMs current with stock, price breaks, lifecycle and alternates from global distributors and South African suppliers, checked on request (`ohmforge bom check`, or in the app) or on a schedule, using your own API keys and working offline from a dated cache, or entered by hand (P3 to P5). The plan is in [`docs/design/parts-availability.md`](docs/design/parts-availability.md).

**Planned: AI link, scenario sweeps and datasheets.** Run thousands of variants of a circuit (tolerances, supply, temperature, load, firmware inputs) headlessly and in parallel, check them against pass/fail rules and graph the operating regions and thresholds; keep every part's datasheet on hand; and connect any AI model, local or cloud, through an MCP server to look up BOM details, run sweeps and review the design with cited datasheet pages. Read-only by default, with every change confirmed by you; no model is bundled (P3 to P5). The plan is in [`docs/design/ai-and-scenario-sweeps.md`](docs/design/ai-and-scenario-sweeps.md).

**Planned: wires and traces.** Set each wire's size in mm² (AWG alongside), material and length, and each PCB trace's width and copper weight; the simulation then includes their resistance, heating and voltage drop and warns when a conductor is overloaded (P3 to P5). The plan is in [`docs/design/interconnects.md`](docs/design/interconnects.md).

**Planned: buses.** I2C, SPI, full UART and CAN (the ESP32's TWAI) on the simulated chip, at transaction level for speed or edge by edge through the circuit so pull-ups, capacitance, baud mismatches and missing termination fail as on the bench; device models such as the BME280 and MPU-6050; protocol decoders with VCD and sigrok export (P3 to P5). The plan is in [`docs/design/buses.md`](docs/design/buses.md).

**Planned: component stress states.** Parts show on the schematic when they approach or exceed their ratings, while running or paused (with a preview of an edit before resuming), with an optional mode in which they fail like real hardware (P3 to P5). The plan is in [`docs/design/component-stress-states.md`](docs/design/component-stress-states.md).

**Planned: schematic themes and auto-layout.** Today's colours stay the default; optional themes (Cyberpunk / Hacker, Wasteland Terminal, Blueprint, Paper and more), IEC symbols by default (ANSI selectable), and automatic placement that cuts wire crossings while following schematic best practice (P3 and P4). The plans are in [`docs/design/schematic-themes.md`](docs/design/schematic-themes.md) and [`docs/design/schematic-layout.md`](docs/design/schematic-layout.md).

**Planned: signal flow and memory.** See where a signal goes inside the ESP32 (a live block diagram of pads, the GPIO matrix and peripherals, lit from the simulated register state) and where it moves on the schematic (logic levels, current dots, and a summary when a signal is too fast to watch), and open a memory map and hex view of SRAM, flash and the ROM once you have fetched it (P3 to P5). The plan is in [`docs/design/signal-flow-visualisation.md`](docs/design/signal-flow-visualisation.md).

**Planned: official examples and file-based sharing.** A gallery of complete, documented projects you can open and edit, and a project pack you can send as a file, with no account (P3; the web app opens the same packs in P4). The plan is in [`docs/design/examples-and-sharing.md`](docs/design/examples-and-sharing.md).

**Planned: compatibility checks and breadboard view.** Cited ratings on each part and a visible Checks panel for wiring and interface mistakes (P3 first slice; full review in P4), and a true 2.54 mm breadboard of the same circuit (P3). The plans are in [`docs/design/compatibility-checks.md`](docs/design/compatibility-checks.md) and [`docs/design/breadboard.md`](docs/design/breadboard.md).

**Planned: KiCad export and wireless fidelity docs.** Schematic, netlist and XLSX BOM export in P3 ([`docs/design/kicad-export.md`](docs/design/kicad-export.md)). The P4 virtual Wi-Fi radio ships with a page that states what it can and cannot do ([`docs/design/wireless.md`](docs/design/wireless.md)).

## Workspace layout

| Crate | Purpose |
|---|---|
| `of-core` | Virtual time (picoseconds), deterministic scheduler, events |
| `of-netlist` | Circuit graph, nets, analog/digital partitioning |
| `of-analog` | MNA solver, Newton-Raphson, sparse LU, adaptive timestep |
| `of-digital` | Event-driven 4-state logic kernel |
| `of-component-api` | Component trait: the engine's component API from phase E2 (RFC 0001, #62), stable from 1.0 |
| `of-components` | Built-in part models |
| `of-parts` | `part.toml` schema and loader |
| `of-circuit` | Circuit files (`*.circuit.toml`, RFC 0003): parse with line and column errors, validate, canonical save (phase E4, #48) |
| `of-cpu-riscv`, `of-cpu-xtensa` | CPU cores (ESP32-C3 first, Xtensa later) |
| `of-cpu-qemu-ipc` | Client for the out-of-process QEMU sidecar (no GPL linking) |
| `of-soc-esp32c3`, `of-soc-esp32` | SoC memory maps and peripherals (ESP32-C3 first) |
| `of-firmware` | ELF loading, ESP flash images and partition tables (DWARF later) |
| `of-plugin-wasm` | Sandboxed WASM component host |
| `of-instruments` | Measurement service and instruments: scope, logic analyser, decoders, multimeter, logger; serial monitor model (`SerialMonitor`, lean console) and the E5a measurement-service core (signals, sample rings, statistics), real-world profiles with ± bands and instrument loading (E5f: Fluke 87V, Rigol DS1054Z, Rigol DP832; data in `parts/instruments/`, chosen in the desktop's Profiles row); E5g: a logic analyser with the Saleae Logic 8 profile, `d(..)` signals (VCD wires in `ohmforge run --log`) and a UART console that decodes a TX pin (`ohmforge run --uart`, the ESP32-C3's U0TXD on GPIO21, and desktop console and logic analyser panels); the instruments to do, phase E5 in P3, #49, RFC 0004. The bench PSU becomes a component (E2) |
| `of-engine` | Base engine: builds components and circuit files into the solvers, checks the circuit, runs the kernel (E2, #62; circuit files E4, #48) |
| `of-bom` | Bill of materials, costing, CSV export |
| `of-kicad` | KiCad netlist and schematic export |
| `of-gdb` | GDB remote protocol |
| `of-ui` | egui views (schematic, breadboard, panels) |
| `of-app` | Desktop app (`--features gui` for the window) |
| `of-web` | WASM entry point |
| `of-cli` | `ohmforge` headless runner |

Other folders: `parts/` (CC-BY-SA-4.0 parts library), `firmware/` (small test firmware, see [`firmware/README.md`](firmware/README.md)), `circuits/` (CC-BY-SA-4.0 circuit files, see [`circuits/README.md`](circuits/README.md)), `scripts/` (riscv-tests, ESP ROM fetch and ngspice golden-data helpers), `docs/` (design, plan, roadmap and UX research), `rfcs/` (design proposals), `assets/` (logo, icons, and the [startup animation](assets/startup/README.md)), `.github/workflows/` (CI: fmt, clippy, cargo-deny, tests on Linux/Windows/macOS, wasm, GUI and riscv-tests).

## Building

Requires Rust 1.95 or newer (edition 2024; egui 0.37 needs 1.95). `rust-toolchain.toml` selects stable with rustfmt, clippy and the `wasm32-unknown-unknown` target.

```sh
cargo check --workspace
cargo test --workspace
cargo run -p of-cli -- --version
cargo run -p of-cli -- run crates/of-analog/tests/circuits/rc_step.cir --out rc.csv   # SPICE netlist to CSV
cargo run -p of-cli -- circuit check circuits/   # validate circuit files (*.circuit.toml, RFC 0003), their probe and instrument signals included
cargo run -p of-cli -- run circuits/demos/blink.circuit.toml --log led.csv   # log the file's probes and instruments (CSV, or .vcd)
cargo run -p of-cli -- validate-parts parts/   # check every part.toml in the parts library
cargo run -p of-cli -- bom dimmer --out bom.csv   # demo-board BOM as CSV (blink, dimmer, fade or button)
cargo test -p of-soc-esp32c3          # blinky, timer-interrupt, XIP and button firmware on the ESP32-C3 model, LED and button co-simulated
scripts/fetch-esp-rom-elfs.sh         # optional: Espressif's ESP32-C3 ROM and espflash's bootloader for the real-ROM boot tests and the pot dimmer / LED fade demos (never committed)
cargo run -p of-app --features gui   # desktop app: load screen, then live blink, dimmer, fade and push-button demos
cargo run -p of-app --features gui -- --no-splash   # skip the EILIi load screen this run (`OF_NO_SPLASH=1` also works; uncheck Show startup animation to persist)
```

On Windows, build from a shell with the MSVC Build Tools environment loaded (see `AGENTS.md` section 9).

New to Ohmforge? [Your first schematic](docs/src/guide/first-schematic.md) previews the planned workflow for creating a new schematic from scratch (coming soon; it does not work yet) and ends with what works today: the demos, SPICE netlists with `ohmforge run`, and adding parts.

## Licence

Code: [GNU AGPL-3.0-or-later](LICENSE). If you distribute Ohmforge or a modified version, or offer it to users over a network, you must share the full source under the same licence. For closed-source commercial use, ask about a separate commercial licence.

Parts data, circuit files, artwork and docs: CC-BY-SA-4.0 ([parts/LICENSE.md](parts/LICENSE.md), [circuits/LICENSE.md](circuits/LICENSE.md)). See [NOTICE](NOTICE).

Contributions are accepted under the Developer Certificate of Origin; see [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing and community

- **Start here:** [CONTRIBUTING.md](CONTRIBUTING.md) covers the ground rules, licence hygiene, the checks to run and how to add a part. Issues labelled [`good first issue`](https://github.com/sjnorval/Ohmforge/labels/good%20first%20issue) are small, well-scoped starting points.
- **Questions and ideas:** [Discussions](https://github.com/sjnorval/Ohmforge/discussions). See [SUPPORT.md](SUPPORT.md) for where to ask what.
- **Bugs, features and part requests:** [open an issue](https://github.com/sjnorval/Ohmforge/issues/new/choose) using one of the forms.
- **Security:** report privately; see [SECURITY.md](SECURITY.md).
- **Conduct:** everyone follows the [Code of Conduct](CODE_OF_CONDUCT.md).
- **What has changed:** [CHANGELOG.md](CHANGELOG.md).

## Roadmap

The planned path from the work already on `main` through to 1.0 is below.

| Phase | Status | What it covers | Release |
| --- | --- | --- | --- |
| P0 Foundations | ✅ Done (leftovers #57, #58) | Repo, CI, RFCs, parts validation | n/a |
| RISC-V core (phase one) | ✅ Done | RV32IMC, 75 of 77 riscv-tests | n/a |
| P1 Analog core | ✅ Done | DC, AC and transient analyses within 1 % of ngspice | n/a |
| P2 ESP32-C3 MVP | 🚧 In progress | Boot, GPIO, ADC, PWM, UART, live demo, BOM | 0.1 |
| P3 Breadth | 📋 Planned | Classic ESP32, I2C/SPI, breadboard, examples and sharing, engine E4–E6 | 0.2 |
| P4 Own Xtensa core and web | 📋 Planned | Own Xtensa core, web app, Wi-Fi, MicroPython on the C3 | 0.3 |
| P5 Realism | 📋 Planned | Monte Carlo, thermal model, fault injection | 0.4 |
| P6 1.0 | 📋 Planned | Stable API, ESP32-S3 and ESP32-C6 | 1.0 |

The full checklist is in [`docs/ROADMAP.md`](docs/ROADMAP.md).
