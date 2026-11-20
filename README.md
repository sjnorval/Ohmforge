<p align="center">
  <img src="assets/social-preview.png" alt="Ohmforge — emulate ESP32 in the browser. No board required." width="700">
</p>

<p align="center">
  <a href="https://github.com/sjnorval/Ohmforge/actions/workflows/ci.yml"><img src="https://github.com/sjnorval/Ohmforge/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-AGPL--3.0--or--later-blue" alt="Licence: AGPL-3.0-or-later"></a>
</p>

An open-source, true-to-life ESP32 circuit simulator written in Rust, with built-in bill-of-materials tracking.

**Status:** pre-alpha. The SPICE-grade analog solver (`of-analog`, checked against ngspice), the RV32IMC CPU core (`of-cpu-riscv`, passing riscv-tests), the co-simulation kernel (`of-core`) and the parts-to-devices bridge (`of-components`) work. The ESP32-C3 SoC (`of-soc-esp32c3`) boots firmware from an ELF or an ESP flash image: a blinky drives a simulated LED and resistor end to end, and timer interrupts and watchdogs behave as on the chip. Espressif's real boot ROM runs on the model and boots firmware from simulated SPI flash through the cache MMU; an unmodified esp-hal blinky boots through the ROM and the ESP-IDF second-stage bootloader and drives the LED, esp-hal firmware reads a simulated push button (polled or on a GPIO interrupt) through realistic input thresholds and pull-ups, and on branch `p2-adc-pwm` it reads a potentiometer through the calibrated ADC and dims and fades an LED with LEDC PWM, every PWM edge reaching the circuit at its exact time. In the desktop demo, the LED lights on the schematic in sync with the simulated current waveform. The app also shows the firmware's UART0 output, a configurable bench supply on the VDD rail, and click-to-place virtual voltmeters and ammeters. The plan is in [`docs/DESIGN_AND_PLAN.md`](docs/DESIGN_AND_PLAN.md); progress in [`docs/ROADMAP.md`](docs/ROADMAP.md); the parts we plan to support in [`docs/COMPONENTS.md`](docs/COMPONENTS.md).

## Goals

- Run unmodified ESP32 firmware (ESP-IDF, Arduino-ESP32, esp-hal, MicroPython) against a SPICE-grade analog circuit, in one deterministic simulation.
- Native desktop app and a WASM web app from one codebase.
- Every part is a real, orderable part, so the BOM, cost (ZAR) and build checklist come straight from the design.

## Workspace layout

| Crate | Purpose |
|---|---|
| `of-core` | Virtual time (picoseconds), deterministic scheduler, events |
| `of-netlist` | Circuit graph, nets, analog/digital partitioning |
| `of-analog` | MNA solver, Newton-Raphson, sparse LU, adaptive timestep |
| `of-digital` | Event-driven 4-state logic kernel |
| `of-component-api` | Stable component/plugin trait |
| `of-components` | Built-in part models |
| `of-parts` | `part.toml` schema and loader |
| `of-cpu-riscv`, `of-cpu-xtensa` | CPU cores (ESP32-C3 first, Xtensa later) |
| `of-cpu-qemu-ipc` | Client for the out-of-process QEMU sidecar (no GPL linking) |
| `of-soc-esp32c3`, `of-soc-esp32` | SoC memory maps and peripherals (ESP32-C3 first) |
| `of-firmware` | ELF loading, ESP flash images and partition tables (DWARF later) |
| `of-plugin-wasm` | Sandboxed WASM component host |
| `of-instruments` | Scope, logic analyser, decoders, multimeter, PSU; serial monitor model (`SerialMonitor`, lean console; scope/LA/DMM/PSU to do) |
| `of-bom` | Bill of materials, costing, CSV export |
| `of-kicad` | KiCad netlist and schematic export |
| `of-gdb` | GDB remote protocol |
| `of-ui` | egui views (schematic, breadboard, panels) |
| `of-app` | Desktop app (`--features gui` for the window) |
| `of-web` | WASM entry point |
| `of-cli` | `ohmforge` headless runner |

Other folders: `parts/` (CC-BY-SA-4.0 parts library), `firmware/` (small test firmware, see [`firmware/README.md`](firmware/README.md)), `scripts/` (riscv-tests, ESP ROM fetch and ngspice golden-data helpers), `docs/` (design, plan, roadmap and UX research), `rfcs/` (design proposals), `assets/` (logo and icons), `.github/workflows/` (CI: fmt, clippy, cargo-deny, tests on Linux/Windows/macOS, wasm, GUI and riscv-tests).

## Building

Requires Rust 1.95 or newer (edition 2024; egui 0.37 needs 1.95). `rust-toolchain.toml` selects stable with rustfmt, clippy and the `wasm32-unknown-unknown` target.

```sh
cargo check --workspace
cargo test --workspace
cargo run -p of-cli -- --version
cargo run -p of-cli -- run crates/of-analog/tests/circuits/rc_step.cir --out rc.csv   # SPICE netlist to CSV
cargo test -p of-soc-esp32c3          # blinky, timer-interrupt, XIP and button firmware on the ESP32-C3 model, LED and button co-simulated
scripts/fetch-esp-rom-elfs.sh         # optional: Espressif's ESP32-C3 ROM and espflash's bootloader for the real-ROM boot tests and the pot dimmer / LED fade demos (never committed)
cargo run -p of-app --features gui   # desktop app: live blink, dimmer, fade and push-button demos
```

On Windows, build from a shell with the MSVC Build Tools environment loaded (see `AGENTS.md` section 9).

## Licence

Code: [GNU AGPL-3.0-or-later](LICENSE). If you distribute Ohmforge or a modified version, or offer it to users over a network, you must share the full source under the same licence. For closed-source commercial use, ask about a separate commercial licence.

Parts data, artwork and docs: CC-BY-SA-4.0 ([parts/LICENSE.md](parts/LICENSE.md)). See [NOTICE](NOTICE).

Contributions are accepted under the Developer Certificate of Origin; see [CONTRIBUTING.md](CONTRIBUTING.md).

## Contributing and community

- **Start here:** [CONTRIBUTING.md](CONTRIBUTING.md) covers the ground rules, licence hygiene, the checks to run and how to add a part. Issues labelled [`good first issue`](https://github.com/sjnorval/Ohmforge/labels/good%20first%20issue) are small, well-scoped starting points.
- **Questions and ideas:** [Discussions](https://github.com/sjnorval/Ohmforge/discussions). See [SUPPORT.md](SUPPORT.md) for where to ask what.
- **Bugs, features and part requests:** [open an issue](https://github.com/sjnorval/Ohmforge/issues/new/choose) using one of the forms.
- **Security:** report privately; see [SECURITY.md](SECURITY.md).
- **Conduct:** everyone follows the [Code of Conduct](CODE_OF_CONDUCT.md).
- **What has changed:** [CHANGELOG.md](CHANGELOG.md).

## Roadmap

The full checklist is in [`docs/ROADMAP.md`](docs/ROADMAP.md).

- **P0 Foundations**: mostly done (workspace, licences, CI, parts loader, kernel `Engine` trait). Leftovers such as the project file format and the mdBook skeleton continue alongside later phases.
- **Phase one, RISC-V core**: done (RV32IMC core, 75 of 77 riscv-tests passing, 2 expected-unsupported).
- **P1 Analog core**: done (sparse MNA with DC, AC and transient analyses, SPICE device models, AMS1117 macromodel, parts bridge; all within 1 % of ngspice).
- **P2 ESP32-C3 MVP**: in progress. Landed: ELF boot, GPIO output, interrupts, timers, watchdogs, the flash image parser, SPI flash, cache MMU, real-ROM boot, an esp-hal blinky through the ROM and the ESP-IDF bootloader, GPIO inputs with a push button, the SAR ADC and LEDC PWM, a potentiometer part, and a live desktop demo (blink, pot dimmer, LED fade and push-button; the esp-hal demos need the fetched ROM) with a configurable bench supply on the schematic and click-to-place virtual voltmeters and ammeters. Next: ESP-IDF and Arduino firmware, the schematic editor, and BOM v1.
- **P3 Breadth**: planned (classic ESP32 through a QEMU sidecar, I2C/SPI/RMT/DAC, breadboard view, about 50 parts, electromechanical models, KiCad export; exit 0.2).
- **P4 Own Xtensa core and web**: planned (own Xtensa core, browser web app, Wi-Fi virtual radio, power profiler, energy parts; exit 0.3).
- **P5 Realism**: planned (Monte Carlo, thermal model, fault injection, hardware-in-the-loop lab, published validation report; exit 0.4).
- **P6 1.0**: planned (stable component API and WASM SDK, ESP32-S3 and ESP32-C6, complete documentation, BLE as a stretch goal).
- **Future ideas (parked)**: optional cloud accounts for sync, and toggleable thermal and electromagnetic interaction layers. These are untracked and need design discussion first.
