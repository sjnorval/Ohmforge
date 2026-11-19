<p align="center">
  <img src="assets/social-preview.png" alt="Ohmforge — emulate ESP32 in the browser. No board required." width="700">
</p>

<p align="center">
  <a href="https://github.com/sjnorval/Ohmforge/actions/workflows/ci.yml"><img src="https://github.com/sjnorval/Ohmforge/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/licence-AGPL--3.0--or--later-blue" alt="Licence: AGPL-3.0-or-later"></a>
</p>

An open-source, true-to-life ESP32 circuit simulator written in Rust, with built-in bill-of-materials tracking.

**Status:** pre-alpha. The SPICE-grade analog solver (`of-analog`, checked against ngspice), the RV32IMC CPU core (`of-cpu-riscv`, passing riscv-tests), the co-simulation kernel (`of-core`) and the parts-to-devices bridge (`of-components`) work. The ESP32-C3 SoC (`of-soc-esp32c3`) boots firmware from an ELF or an ESP flash image: a blinky drives a simulated LED and resistor end to end, and timer interrupts and watchdogs behave as on the chip. In the desktop demo, the LED lights on the schematic in sync with the simulated current waveform. The app also shows the firmware's UART0 output. The plan is in [`docs/DESIGN_AND_PLAN.md`](docs/DESIGN_AND_PLAN.md); progress in [`docs/ROADMAP.md`](docs/ROADMAP.md).

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
| `of-firmware` | ELF loading (flash images and DWARF later) |
| `of-plugin-wasm` | Sandboxed WASM component host |
| `of-instruments` | Scope, logic analyser, decoders, multimeter, PSU |
| `of-bom` | Bill of materials, costing, CSV export |
| `of-kicad` | KiCad netlist and schematic export |
| `of-gdb` | GDB remote protocol |
| `of-ui` | egui views (schematic, breadboard, panels) |
| `of-app` | Desktop app (`--features gui` for the window) |
| `of-web` | WASM entry point |
| `of-cli` | `ohmforge` headless runner |

Other folders: `parts/` (CC-BY-SA-4.0 parts library), `firmware/` (small test firmware, see [`firmware/README.md`](firmware/README.md)), `scripts/` (riscv-tests and ngspice golden-data helpers), `docs/` (design, plan, roadmap and UX research), `rfcs/` (design proposals), `assets/` (logo and icons), `.github/workflows/` (CI: fmt, clippy, cargo-deny, tests on Linux/Windows/macOS, wasm, GUI and riscv-tests).

## Building

Requires Rust 1.95 or newer (edition 2024; egui 0.37 needs 1.95). `rust-toolchain.toml` selects stable with rustfmt, clippy and the `wasm32-unknown-unknown` target.

```sh
cargo check --workspace
cargo test --workspace
cargo run -p of-cli -- --version
cargo run -p of-cli -- run crates/of-analog/tests/circuits/rc_step.cir --out rc.csv   # SPICE netlist to CSV
cargo test -p of-soc-esp32c3          # blinky and timer-interrupt firmware on the ESP32-C3 model, LED co-simulated
cargo run -p of-app --features gui   # desktop app with the ESP32-C3 LED blink demo
```

On Windows, build from a shell with the MSVC Build Tools environment loaded (see `AGENTS.md` section 9).

## Licence

Code: [GNU AGPL-3.0-or-later](LICENSE). If you distribute Ohmforge or a modified version, or offer it to users over a network, you must share the full source under the same licence. For closed-source commercial use, ask about a separate commercial licence.

Parts data, artwork and docs: CC-BY-SA-4.0 ([parts/LICENSE.md](parts/LICENSE.md)). See [NOTICE](NOTICE).

Contributions are accepted under the Developer Certificate of Origin; see [CONTRIBUTING.md](CONTRIBUTING.md).


## Development status

- **P1 Analog core**: done (sparse MNA with DC/AC/transient analyses, SPICE device models, AMS1117 macromodel, parts bridge; all within 1 % of ngspice).
- **Phase one, RISC-V core**: done (RV32IMC(+A), 75 of 77 riscv-tests).
- **P2 ESP32-C3 MVP**: in progress. Done so far: ELF and ESP flash image loading, the C3 memory map, GPIO/IO MUX output, UART0 transmit, SYSTIMER alarms, the interrupt matrix, timer groups and watchdogs, and the chip running under the kernel next to the analog solver. See [`docs/ROADMAP.md`](docs/ROADMAP.md).
