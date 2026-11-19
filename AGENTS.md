# AGENTS.md: instructions for coding agents

This file is the cross-tool agent guide (read by Grok Build, Codex, Cursor and others). Grok Build also loads `.grok/rules/*.md`.

## 1. What this project is

**Ohmforge** is an open-source, true-to-life **ESP32 circuit simulator in Rust**. It runs unmodified ESP32 firmware (ESP-IDF, Arduino-ESP32, esp-hal, MicroPython ELF files) against a SPICE-grade analog circuit in one deterministic simulation. Every placed part is a real, orderable part, so the bill of materials (MPN, supplier, ZAR cost) and a real-world build checklist come straight from the design. It targets a native desktop app and a WASM web app from one codebase.

**Source of truth:** [`docs/DESIGN_AND_PLAN.md`](docs/DESIGN_AND_PLAN.md). Read it before planning anything non-trivial. If your change contradicts it, stop and propose an update to the plan (or an RFC in `rfcs/`) instead of silently diverging. Do not edit `docs/DESIGN_AND_PLAN.md` unless the owner asks.

**Current status and next tasks:** [`docs/ROADMAP.md`](docs/ROADMAP.md). Pick work from the current phase, tick boxes only when the work is verified and committed, and keep that file up to date.

**Current phase: P2 (ESP32-C3 MVP) is in progress.** P1 (analog core) is complete: the solver (`crates/of-analog`, the SPICE parser in `of-netlist::spice`, `ohmforge run`) matches ngspice within 1 % on the reference set; parts are bridged to solver devices (`of_components::analog`), the solver runs under the `of-core` kernel (`of_analog::AnalogEngine`), and an AMS1117-3.3 macromodel and the MVP board checks are in (section 3b). Phase one, the RISC-V CPU core (`crates/of-cpu-riscv`), passes the official riscv-tests (section 3a). The first P2 slice is on `main`: ELF loading (`of-firmware`), the C3 memory map with GPIO/IO MUX, UART0 TX and SYSTIMER (`of-soc-esp32c3`), the SoC as a kernel `Engine`, and a blinky firmware that drives a parts-library LED end to end. The interrupt matrix, SYSTIMER alarms, timer groups, the watchdogs (MWDT, RTC WDT, super watchdog) and the ESP flash image parser follow, with a timer-interrupt firmware (section 3c). See the roadmap for the checklists.

## 2. Architecture in one minute

- One scheduler (`of-core`) owns virtual time as a `u64` picosecond clock; every engine advances under it, so runs are reproducible.
- CPU cores execute firmware in instruction quanta and stop early on any I/O register access; the analog solver advances to that exact instant first.
- Nets with only digital endpoints are solved by the event-driven logic kernel (`of-digital`); any net touching an analog part is promoted to the MNA solver (`of-analog`).
- ESP32-C3 (RISC-V) runs on our own core first. Classic ESP32/S3 (Xtensa) uses Espressif's QEMU as a **separate GPL process** through `of-cpu-qemu-ipc` until our own Xtensa core exists.
- Parts are data (`parts/**/part.toml`, parsed by `of-parts`) plus behaviour (Rust in `of-components`, or sandboxed WASM via `of-plugin-wasm`) implementing the `Component` trait in `of-component-api`.

## 3. Crate map

| Crate | Responsibility | State |
|---|---|---|
| `of-core` | Virtual time, deterministic scheduler, events; `Engine` trait and `Kernel` (quantum slices, breakpoints, host stops) | Implemented + tests |
| `of-netlist` | Circuit graph, nets, analog/digital partitioning; SPICE netlist subset parser (`spice`) | Net types; SPICE parser implemented |
| `of-analog` | MNA, Newton-Raphson, sparse LU (faer), trapezoidal/Gear-2, LTE timestep, device models | Implemented: DC op, DC sweeps (nested), AC, transient; R C L K V I E G F H (incl. POLY) S W, diode/LED, BJT (full SPICE3 Gummel-Poon), MOSFET L1/L3 with Meyer caps; temperature; `PinDriver`; `AnalogEngine` for the kernel; ngspice-checked |
| `of-digital` | 4-state event-driven logic kernel | Logic type + test |
| `of-component-api` | Stable `Component` trait, `Health`, `FaultKind` | Draft API |
| `of-components` | Built-in part models; `analog`: parts-to-devices bridge (`Library`, `Board`, `build`, `to_spice`) | Bridge implemented + tests |
| `of-parts` | `part.toml` schema (pins, `[analog]`, tolerances/corners) and loader | Implemented + tests |
| `of-cpu-riscv` | RV32IMC(+A) core, Zicsr, M-mode traps/interrupts, cycle timing, debugger API | Implemented: 63 unit tests, 75/77 riscv-tests |
| `of-cpu-xtensa` | Xtensa LX6/LX7 core (phase 4) | Empty |
| `of-cpu-qemu-ipc` | Client for the out-of-process QEMU sidecar (talks over IPC; never links QEMU) | Empty |
| `of-soc-esp32c3` | ESP32-C3 memory map, peripherals (GPIO/IO MUX, UART0 TX, SYSTIMER, interrupt matrix, TIMG0/1 with MWDT, RTC WDT and super watchdog), direct ELF and flash-image boot, `Esp32c3Engine` for the kernel | Implemented + tests (section 3c) |
| `of-soc-esp32` | Classic ESP32 SoC (phase 4) | Empty |
| `of-firmware` | ELF, flash images, UF2, DWARF | ELF32 RISC-V reader (segments, symbols), ESP image parser/writer (checksum, SHA-256) + tests |
| `of-plugin-wasm` | Sandboxed WASM component host | Empty |
| `of-instruments` | Scope, logic analyser, decoders, DMM, PSU | Empty |
| `of-bom` | BOM aggregation, ZAR costing, CSV export | Working + test |
| `of-kicad` | KiCad netlist and `.kicad_sch` export | Empty |
| `of-gdb` | GDB remote serial protocol | Empty |
| `of-ui` | egui views and instrument panels; egui only with `--features gui` | ESP32-C3 blink demo (schematic, LED current trace, UART0) |
| `of-app` | Desktop binary; eframe (wgpu) and `of-ui/gui` only with `--features gui` | Runs the of-ui blink demo |
| `of-web` | WASM entry point | Empty |
| `of-cli` | `ohmforge` headless runner | `ohmforge run netlist.cir` (CSV/VCD) |

Internal dependencies point downwards only: `of-core` depends on nothing internal; UI crates may depend on engines, never the reverse. Simulation crates must stay free of UI and OS dependencies and must build for `wasm32-unknown-unknown`.

## 3a. `of-cpu-riscv` design notes

- **Modules:** `bus` (the `Bus` trait plus a `Ram` test device), `decode` (32-bit decoder, `Instr`, disassembler via `Display`), `compressed` (RVC to 32-bit expander), `csr` (machine CSR file with WARL masking), `trap` (exception codes, `mcause`, interrupt selection), `timing` (`TimingModel` cycle costs), `hart` (`Hart`, `HartConfig`, execution), `asm` (instruction encoders for tests and tools).
- **ISA:** RV32I + M + C + Zicsr + Zifencei; A is optional (`Extensions { a }`: off for the C3, on for the C6 and the riscv-tests run). Machine mode only; `mstatus.MPP` is hard-wired to M. No U-mode, PMP or debug triggers yet.
- **Bus contract:** `read`/`write` with `Size` and `AccessKind`; `wait_states` adds cycles; `csr_read`/`csr_write` let the SoC add custom CSRs (unknown CSRs fall through to the bus, otherwise illegal instruction); `take_sync_request` lets a peripheral access end the current quantum so the analog solver can catch up.
- **Time:** `Hart::local_time()` = time origin + elapsed cycles x clock period (6250 ps at 160 MHz). `run_until(bus, SimTime)` runs a quantum; `idle_until` lets time pass in `wfi`. `elapsed_cycles` is monotonic; `mcycle`/`minstret` are the software-visible counters (writable, `mcountinhibit` honoured).
- **Debugger/GDB:** `step` returns `StepOutcome` (retired/trapped/waiting/halted); `run` returns `RunExit` (budget, breakpoint, halted, WFI, sync). Breakpoints are a `BTreeSet<u32>`, skipped on the first instruction of a run so resume works. `EbreakAction::Halt` turns `ebreak` into a debugger stop. `disassemble_at`, `reg`/`set_reg`, `pc`/`set_pc`, `csr_peek`/`csr_poke`, `halt`/`resume`/`reset`.
- **Interrupts:** the SoC drives lines with `set_interrupt_line`/`set_interrupt_lines` (`mip` is read-only to software). C3 mask is lines 1 to 31; order is MEI, MSI, MTI, then lowest line (the C3 interrupt matrix applies priority/threshold before the core and presents only the winning line). `HartConfig::mie_mip` is false for the C3: like the real chip it has no `mie`/`mip` CSRs (accessing them is an illegal instruction) and a pending line is taken whenever `mstatus.MIE` is set; `generic_imac` keeps both CSRs for the riscv-tests. Direct and vectored `mtvec`.
- **Timing numbers** in `TimingModel::ESP32C3_ESTIMATE` are estimates until calibrated on hardware; do not present them as measured.
- **Compliance:** `tests/riscv_tests.rs` runs riscv-tests ELFs (BSD-3-Clause) from `target/riscv-tests` (or `OF_RISCV_TESTS_DIR`) and skips if absent. Build them with `scripts/build-riscv-tests.sh` or `.ps1` (pinned commit, zig/gcc/clang). Never commit the binaries. `rv32mi-p-breakpoint` (triggers) and `rv32mi-p-pmpaddr` (PMP) are listed as expected-unsupported.

## 3b. `of-analog` design notes

- **Modules:** `matrix` (sparse MNA system on faer sparse LU, plus the complex AC solve), `circuit` (`Circuit`, `Device` trait, `Setup`, `LoadCtx`), `devices` (`passive` incl. `MutualInductance`, `sources`, `switch`, `diode` incl. LED, `bjt`, `mosfet`, `limit` incl. temperature helpers), `integrate` (`Method`, `States`, `Integrator`), `waveform` (PULSE/SIN/EXP/PWL), `analysis` (`dc` incl. `dc_sweep_nested`, `ac`, `newton`, `tran`), `results` (`OpPoint`, `Waveforms`, `AcResult`, CSV/VCD), `netlist` (`build(&Deck)`, `add_deck` to add a deck to an existing circuit), `engine` (`AnalogEngine`).
- **Unknowns:** variable 0 is ground; node voltages first, then internal nodes (`v(dev#suffix)`, for RS/RB/RD, ESR/ESL, `rser` etc.) and branch currents `i(name)` for V sources, inductors, E and H (internal branches such as `i(c1#esl)` are hidden). Output omits internal nodes. Current signs follow SPICE: `i(v1)` flows into the + terminal.
- **Device contract:** `setup` claims matrix slots (`Setup::slot`/`slots2`), branches, internal nodes and state slots once; `load` stamps the Newton companion model every iteration (matrix entries plus RHS `i - g*v`). Charges are integrated via `LoadCtx::integrate(k, cap)`, which marks state `k` for the truncation-error estimate. Devices report limiting (`cx.limited`) so the iteration is not counted as converged, and `next_breakpoint` for source corners. Optional hooks: `ac_stimulus` (AC sources) and `probes` (derived outputs such as an LED's `popt(d1)` in W and `bright(d1)` = I/IF, appended to OP and sweep results). Setup priority: 0 for branch owners, 1 default, 2 for `K` (needs both inductor branches).
- **Matrix:** pattern fixed at `finalize`; symbolic LU computed once; numeric refactorisation skipped when values are unchanged (linear circuits after the first step). No rayon: single-threaded and deterministic, wasm32-friendly. A non-finite solution is reported as a singular matrix naming the suspect unknown.
- **DC:** Newton (`itl1`) with SPICE3 pnjlim/fetlim/limvds, then damped Newton, gmin stepping (diagonal shunt from `gmin*10^10` down by decades, with back-off) and source stepping (both use `itl2`, as in SPICE). The transient operating point uses waveform values at t = 0; `.op` uses the DC value. Nested `.dc` restarts the inner sweep for each outer value and adds a column named after the outer source.
- **AC:** after the DC operating point and a `TranInit` load, every device is loaded in `Mode::Tran` with `small_signal = true` (no limiting, no state advance) using `Integrator::small_signal(ag0)`: `ag0 = 0` gives G and `ag0 = 1` gives G + C, so C is the difference on the same CSC pattern. Each frequency solves `(G + jωC) x = b` with the cached symbolic LU. Devices need no AC-specific code, but must stamp operating-point capacitances when `small_signal` is set (MOSFET Meyer uses 2x the half-cap, as SPICE's INITSMSIG). BJT excess phase is transient-only.
- **Transient:** first step and the step after every breakpoint are backward Euler, then trapezoidal or Gear-2 (`.options method=gear`). Step control is the SPICE3 `CKTterr` charge-based estimate (divided differences of every integrated charge, `trtol` 7, `chgtol` 1e-14); reject below 0.9 h, grow at most 2x; Newton failure divides h by 8. Max step `min(tstep, span/50)` unless `tmax` is given. Breakpoints land exactly; after one, h = 0.1 x min(h, gap to the next). `Integrator::h_prev` is the previous step (BJT excess phase). Switches have no timestep control yet: give switching circuits a `tmax`. The step loop is `analysis::tran::Stepper`, shared by `transient()` and the engine.
- **Models** follow the published SPICE3/ngspice equations: diode (Shockley, breakdown knee with NBV/TCV, depletion/diffusion charge); Gummel-Poon BJT (`qb` with Early and high injection, XTF/VTF/ITF, IRB/RBM, XCJC, CJS to the substrate node, PTF excess phase); MOS1 Shichman-Hodges and MOS3 (ngspice's corrected form, i.e. not `badmos3`) with mode swap, bulk diodes and depletion caps, Meyer caps from TOX; SW/CSW switches with hysteresis; `K` as `M = k·sqrt(L1·L2)`. Temperature: devices take `with_temp(K)`; the builder uses `.temp`/`.options temp` (°C, default 27), `.options tnom`, model `tnom` and instance `temp`/`dtemp`; passives scale by `1 + tc1·dT + tc2·dT²`. Missing MOS KP/PHI/GAMMA/VTO are derived from TOX/UO/NSUB/NSS/TPG as in SPICE. Unsupported model parameters produce a build warning, never silent wrong results; MOSFET `LEVEL` other than 1 or 3 is an error.
- **Engine (`AnalogEngine`, kernel side in `of_core::engine`):** the kernel cuts time into slices ending at the earliest of now + quantum, the next event, breakpoint, the run end and every engine's `next_event`; engines advance in registration order and an early stop shrinks the slice for the later ones, so **register CPUs first and the analog engine last** (a CPU may run up to one quantum ahead; MMIO accesses end its quantum). The analog engine keeps integration history across slices (no restart at slice ends); events map codes to `SetParam` changes (`on_code`) and force a backward-Euler restart (`discontinuity`); `watch(signal, level, edge, target, code)` refines the step down to the crossing within `EngineConfig::resolution` (100 ps default) and posts an event at that time. GPIO outputs are `PinDriver`s (Norton source, `v`/`r` parameters, Hi-Z = 1 TΩ). `value(name)` reads any unknown or probe; `record` keeps `Waveforms`.
- **POLY sources:** `E`/`G` with `POLY(n)` (n ≤ 8) use the SPICE2 term order (constant, linear, then products by degree in lexicographic index order) with the exact Jacobian. ngspice accepts them natively, but its XSPICE code drops `trtol` to 1 when they are present, so golden decks using them set `.options trtol=1`.
- **Parts bridge (`of_components::analog`):** a `part.toml` gives `[[pin]]` (number, name) and `[analog]` (`kind` = resistor, capacitor, led, diode, npn, pnp, nmos, pmos, load or subckt; `pins` in device order; `params` as plain values or `{ value/typical, min, max, tolerance }`; `file`/`subckt` for subcircuit parts). `Placement::new(refdes, part, &[(pin, net)])` with `.with(param, value)` overrides; a `Board` has placements, a `Corner` (Typical, Min, Max, Deviation(k)) and `Env` (temp/tnom). Overrides keep the part's relative spread at a corner. Primitive parts become devices named by lower-case refdes; subcircuit parts are flattened per instance as `x<refdes>` (so `v(xu1.tj)`); net `0`/`gnd` is ground; every pin must be connected. `to_spice` writes the same board as a SPICE deck. Ground truth for new analog parts: a datasheet test through the bridge plus an ngspice golden circuit.
- **AMS1117 macromodel** (`parts/regulators/ams1117/ams1117.lib`, our own, CC-BY-SA-4.0): standard primitives only (depletion-NMOS quiescent current, ideal reference with supply coupling, Gm error amplifier with clamps, PNP driver into an NPN pass device, sense resistor and current limiter, POLY power into an `Rth`/`Cth` thermal node with a thermal limit near `tsd`). `v(x.tj)` is the rise over `tamb`. Matches the datasheet (Vout, line/load regulation, Iq ≈ 5 mA, dropout ≈ 1.1 V at 0.8 A, limit ≈ 1.1 A, ripple rejection ≈ 72 dB at 120 Hz) and ngspice (`ams1117_*` circuits). Not modelled: the ADJ variant, behaviour below about 1.5 V in, and electro-thermal coupling into other devices.
- **Validation:** `tests/analytic.rs` (closed-form answers) and `tests/ngspice_compare.rs` (circuits in `tests/circuits`, golden CSVs in `tests/golden`, regenerated with `scripts/gen-ngspice-golden.sh [name ...]`). ngspice is run only as an external program to produce data: never link, vendor, port or ship it (its tree contains a GPL file). Ohmforge-only syntax (ESR/ESL, `rser`, `led` models) is rewritten for the reference run with `* ngspice-replace: FROM => TO` lines. Limits: 1 % of the reference signal's span (tran/DC); AC magnitude 1 % per point (floor 1e-3 of the peak) and phase 1 % of 180°; nested DC row by row. Fast-edge circuits set `tmax` so both simulators resolve the edges. `.include` paths are relative to the circuit file (the golden script rewrites them for ngspice). `of-components/tests` holds the bridge tests, AMS1117 datasheet checks, the MVP board built from parts (compared with the `mvp_board` golden) and a scripted-CPU co-simulation.

## 3c. `of-soc-esp32c3` design notes

- **Modules:** `map` (address map constants from the TRM), `bus` (`SocBus`: memories plus peripheral dispatch, implements `of_cpu_riscv::Bus`), `gpio` (GPIO, GPIO matrix output select, IO MUX `MCU_SEL`/`FUN_DRV`, `PinOut`), `uart` (UART0 TX capture), `systimer` (two 16 MHz units, three comparators, interrupts), `intmatrix` (`IntMatrix`: source map, CPU interrupt controller enable/type/priority/threshold/clear), `timg` (TIMG0/1: timer 0 and MWDT), `rtc_cntl` (RTC WDT, super watchdog, RTC interrupt registers), `watchdog` (shared stage logic, `ResetKind`, `ResetSource`, `WatchdogReset`), `soc` (`Esp32c3`: hart + bus, `load_elf`, `load_flash_image`, `run_slice`/`run_until`, `resets`, time-stamped `PinEdge`s and `UartByte`s), `engine` (`Esp32c3Engine`, `PinLink`, `pin_code`).
- **Memory map:** internal ROM (IBus 0x4000_0000, 384 KiB; ROM1 also at DBus 0x3ff0_0000), SRAM through IRAM 0x4037_c000 and DRAM 0x3fc7_c000 (same bytes; DRAM = IRAM - 0x70_0000), flash cache windows IROM 0x4200_0000 and DROM 0x3c00_0000 (separate backing, no MMU yet: each window is filled from the ELF segments at the same address), RTC fast 0x5000_0000, peripherals 0x6000_0000 to 0x600d_0fff. Fetch only from IRAM, IROM and ROM; ROM and flash are read-only to the hart (loaders use `SocBus::load`).
- **Boot:** direct ELF boot (`Esp32c3::load_elf` copies segments, zero-fills `.bss`, sets the PC to `e_entry`) or from a parsed ESP flash image (`load_flash_image(&EspImage)`: checks chip ID 5, skips address-0 padding segments, loads segments like the ROM's direct copy, no cache MMU). Watchdogs start disabled; `arm_boot_watchdogs()` gives the state the ROM leaves on a flash boot (MWDT0 stage 0 system reset after about 325 ms, RWDT, super watchdog after 1 s). A watchdog reset (logged in `resets()`) resets the hart and peripherals and reboots the loaded image, re-arming boot watchdogs unless it was a CPU-only reset. Booting through the real ROM (esp-rom-elfs) comes later.
- **Peripherals:** every peripheral access sets a sync request; `Esp32c3::run_slice` then stamps pin changes and UART bytes with `hart.local_time()` (end of the instruction) and latches SYSTIMER. Writes to timed blocks (SYSTIMER, TIMG, RTC_CNTL) are applied at that same instant (`SocBus::settle`); interrupt-matrix writes apply at once. Slices also end at `SocBus::next_event()` (the next alarm or watchdog stage), so interrupts and resets happen at their exact time, and `wfi` idles to the next event. Interrupt sources are recomputed after every sync and the matrix hands the hart a single line. Not modelled yet: the controller's few-cycle latency, SYSTIMER 52-bit wrap and stall-on-halt, the super watchdog's reset flag, and GPIO interrupts. Unmodelled peripheral registers read back the last write and are counted in `SocBus::unmodelled()`: check it when new firmware misbehaves. GPIO input reads see the level from the last sync (pre-access stops come with the mixed-signal sync item).
- **Pins to analog:** `Esp32c3Engine` turns changes on linked pins into kernel events (`pin_code`: pin, drive strength, low/high/Hi-Z) at the exact hart time and returns early there; register it before the analog engine and map every code from `all_pin_codes(pin)` to `PinDriver` `v`/`r` with `PinOut::thevenin` (output resistance about 120/60/30/15 Ω per drive strength: an estimate, not a datasheet value).
- **Test firmware:** `firmware/esp32c3-blinky` (`blinky.S`, polled SYSTIMER) and `firmware/esp32c3-timer-irq` (`timer_irq.S`: disables the watchdogs, SYSTIMER period alarm and TIMG0 auto-reload alarm through the interrupt matrix into vectored handlers, `wfi` between them), each with `link.ld` and `build.sh` (overview in `firmware/README.md`): bare-metal RV32IMC assembly built with zig (`pip install ziglang==0.17.1`). The small ELFs (1.5 KB and 2.4 KB) are committed (mode 644; `build.sh` resets it because lld marks outputs executable) so tests need no toolchain; CI rebuilds them and fails if they differ in content or mode. Rebuild with the firmware's `build.sh` after editing the source and commit both. Test firmware is our own code (AGPL-3.0-or-later); never commit third-party binaries.

## 4. Coding conventions

- Rust **edition 2024**, **MSRV 1.95** (egui 0.37 needs it). Toolchain pinned to stable via `rust-toolchain.toml`.
- Formatting: `cargo fmt --all` (config in `rustfmt.toml`, max width 120).
- Lints: `cargo clippy --workspace --all-targets -- -D warnings` must be clean. Workspace lints deny `unsafe_code` and warn on `missing_docs`; every public item gets a doc comment.
- Tests: every new behaviour gets unit tests in the same crate; numeric code compares with tolerances, never exact floats. Prefer small deterministic tests; analog results are validated against closed-form answers and ngspice (section 3b).
- Dependencies: add them once in the root `[workspace.dependencies]` and use `x.workspace = true` in crates. Keep heavy dependencies (egui, wgpu, wasmtime) behind the crates or features that need them.
- Errors: `thiserror` in libraries; no `unwrap()` in library code outside tests.
- Units: SI units in `f64` for analog values; simulation time is always `of_core::SimTime` (picoseconds), never wall-clock.
- Determinism: no `HashMap` iteration order, threads, or wall-clock time influencing simulation results.

## 5. Licensing rules (hard constraints)

- Code is **AGPL-3.0-or-later** (`LICENSE`; the workspace `license` field, inherited by every crate). Parts data, artwork and docs are **CC-BY-SA-4.0**; every `part.toml` needs a `[license]` block with `data = "CC-BY-SA-4.0"` and `artwork = "CC-BY-SA-4.0"`, and model files under `parts/` carry the same licence line. The copyright holder may offer a separate commercial licence (see `NOTICE`).
- **Dependencies must be AGPL-compatible:** permissive licences on the `deny.toml` allow list. Never add GPL-2.0-only, LGPL or other copyleft dependencies, and never widen the allow list to GPL without Stefan's approval. Run `cargo deny check` if cargo-deny is installed (CI always runs it).
- Never copy or translate code from QEMU, Falstad CircuitJS, SimulIDE (GPL) or Wokwi (proprietary). Ideas and published algorithms only.
- QEMU only ever runs as a separate process; never link or `dlopen` it.
- Do not commit vendor SPICE models with restrictive terms, KiCad library files or datasheets; link to them.
- ngspice stays an external validation tool only: never link, vendor or ship it (section 3b).

## 6. Build and test (Windows, PowerShell)

```powershell
cd E:\Work\2026\Projects\Circuit   # or D:\Experiments\Circuit on NorvalS
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p of-app --features gui
cargo check --target wasm32-unknown-unknown -p of-core -p of-analog -p of-digital -p of-netlist -p of-parts -p of-bom -p of-web -p of-cpu-riscv -p of-components -p of-firmware -p of-soc-esp32c3
.\scripts\build-riscv-tests.ps1                  # optional: fetch and build riscv-tests (needs zig or a RISC-V gcc)
cargo test -p of-cpu-riscv --test riscv_tests -- --nocapture
cargo test -p of-soc-esp32c3                     # blinky, timer interrupts and watchdogs on the SoC; blinky co-simulated with an LED
bash firmware/esp32c3-blinky/build.sh            # optional: rebuild the test firmware (needs zig, `pip install ziglang==0.17.1`, and bash, e.g. Git Bash)
bash firmware/esp32c3-timer-irq/build.sh
cargo run -p of-cli -- --version
cargo test -p of-analog --test ngspice_compare -- --nocapture   # prints the ngspice comparison table
cargo run -p of-cli -- run crates/of-analog/tests/circuits/rc_step.cir --out rc.csv
cargo run -p of-app --features gui      # desktop app with the ESP32-C3 LED blink demo (click Play)
```

Run all of these before proposing a commit; they mirror `.github/workflows/ci.yml`. **Load the MSVC Build Tools environment first** (see section 9), otherwise linking fails.

## 7. Git rules

- **Do not commit without Stefan's approval.** Leave changes in the working tree, report what changed, and propose a commit breakdown with messages; commit only what he approves.
- The remote is `origin` = <https://github.com/sjnorval/Ohmforge>. `main` holds the merged P1 work (PR #7) and the blinky milestone (PR #8); new work goes on a feature branch (currently `p2-interrupts-timers`) and reaches `main` only through a pull request that Stefan merges. Never push to `main`. **Push only when Stefan explicitly asks**, never force-push, and never handle tokens (use the machine's existing git credentials). Before a push, check that no secrets or build artefacts are tracked.
- Every commit is signed off: `git commit -s`.
- Conventional Commits: `feat:`, `fix:`, `docs:`, `test:`, `refactor:`, `chore:`; small, logical commits.
- Never rewrite or force-change existing history.

## 8. Project name

**Ohmforge** is the final project name; crates use the `of-` prefix (`of_` in Rust paths) and the CLI binary is `ohmforge`.

## 9. Windows environment notes

- Machines (shared remote: `origin` on GitHub, see section 7):
  - **Home PC `DESKTOP-TLVLSJ8`**: repo at `E:\Work\2026\Projects\Circuit`, Windows PowerShell 5.1. MSVC comes from **Visual Studio Build Tools 2026** (`...\Microsoft Visual Studio\18\BuildTools`, MSVC 14.51, Windows SDK 10.0.26100), which links fine.
  - **Work PC `NorvalS`**: repo at `D:\Experiments\Circuit`, PowerShell 7. Use **Visual Studio 2022 Build Tools**; its VS 2026 Enterprise `link.exe` lacks `msvcrt.lib` and fails with `LNK1104`.
- Rust via rustup (`stable-x86_64-pc-windows-msvc`), with rustfmt, clippy and the `wasm32-unknown-unknown` target. If `cargo` is not found in a fresh shell, use `$env:USERPROFILE\.cargo\bin\cargo.exe` or open a new terminal.
- Load the machine's Build Tools environment into the PowerShell session before building (set `$vcvars` to the line for that machine):

  ```powershell
  $vcvars = "C:\Program Files (x86)\Microsoft Visual Studio\18\BuildTools\VC\Auxiliary\Build\vcvars64.bat"    # home PC
  # $vcvars = "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\VC\Auxiliary\Build\vcvars64.bat" # NorvalS
  cmd /c "`"$vcvars`" >nul && set" | ForEach-Object { if ($_ -match '^([^=]+)=(.*)$') { Set-Item "env:$($matches[1])" $matches[2] } }
  $env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"
  ```

  Do not switch to the GNU toolchain to dodge this.
- `.gitattributes` normalises text to LF (PowerShell scripts CRLF). Keep paths short; avoid spaces in new paths.
- Grok Build loads project rules only when the folder is trusted (`grok --trust` or the interactive prompt). Check what was loaded with `grok inspect`.
