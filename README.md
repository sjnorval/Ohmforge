# Ohmforge

> **Working name.** An open-source, true-to-life ESP32 circuit simulator written in Rust, with built-in bill-of-materials tracking.

**Status:** pre-alpha scaffold. Nothing simulates yet; the crates are compiling stubs that follow the plan in [`docs/DESIGN_AND_PLAN.md`](docs/DESIGN_AND_PLAN.md).

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
| `of-soc-esp32c3`, `of-soc-esp32` | SoC memory maps and peripherals |
| `of-firmware` | ELF, flash image and DWARF loading |
| `of-plugin-wasm` | Sandboxed WASM component host |
| `of-instruments` | Scope, logic analyser, decoders, multimeter, PSU |
| `of-bom` | Bill of materials, costing, CSV export |
| `of-kicad` | KiCad netlist and schematic export |
| `of-gdb` | GDB remote protocol |
| `of-ui` | egui views (schematic, breadboard, panels) |
| `of-app` | Desktop app (`--features gui` for the window) |
| `of-web` | WASM entry point |
| `of-cli` | `ohmforge` headless runner |

Other folders: `parts/` (CC-BY-4.0 parts library), `docs/` (design and plan), `.github/workflows/` (CI, inert until pushed).

## Building

Requires Rust 1.95 or newer (edition 2024; egui 0.37 needs 1.95). `rust-toolchain.toml` selects stable with rustfmt, clippy and the `wasm32-unknown-unknown` target.

```sh
cargo check --workspace
cargo test --workspace
cargo run -p of-cli -- --version
cargo run -p of-app --features gui   # desktop window (pulls in egui)
```

## Renaming the project

"Ohmforge" and the `of-` crate prefix are placeholders. To rename, replace the strings `ohmforge`, `Ohmforge`, `of-` and `of_` across the repo and rename the `crates/of-*` folders. `scripts/rename.ps1` does this in one go (review the diff before committing).

## Licence

Code: MIT OR Apache-2.0, at your option ([LICENSE-MIT](LICENSE-MIT), [LICENSE-APACHE](LICENSE-APACHE)). Parts data and artwork: CC-BY-4.0 ([parts/LICENSE.md](parts/LICENSE.md)). See [NOTICE](NOTICE).

Contributions are accepted under the Developer Certificate of Origin; see [CONTRIBUTING.md](CONTRIBUTING.md).
