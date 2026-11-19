# Roadmap and task list

Derived from `docs/DESIGN_AND_PLAN.md` section 14. Tick a box only when the work is verified, all checks in `AGENTS.md` pass and it is committed; `[~]` means partly done (what is missing is noted on the item). Work on a feature branch counts once it is pushed with green CI. Status as of 18 November 2026 (`main` has P1 and the blinky milestone; interrupts, timers and watchdogs are on branch `p2-interrupts-timers`).

**Current phase: P2 ESP32-C3 MVP, in progress** (ESP32-C3 SoC on the RISC-V core, co-simulated with the analog solver through the `of-core` kernel). P1 Analog core and phase one (the RISC-V CPU core, pulled forward from P2) are done; the first P2 slice (blinky end to end) is on `main`, and the interrupt matrix, SYSTIMER alarms, timer groups, watchdogs and the ESP image parser follow. P0 leftovers continue alongside.

## Phase one: RISC-V CPU core (`of-cpu-riscv`)

Status: done; the core is implemented and tested (63 unit tests, 1 doc test, 75 of 77 riscv-tests passing, 2 expected-unsupported). Design notes: `AGENTS.md` section 3a.

- [x] `Bus` trait (sizes, access kinds, wait states, custom CSR hooks, sync requests) and a `Ram` device
- [x] RV32I decoder and disassembler; RVC expander (all RV32C forms, reserved encodings rejected)
- [x] Execution of RV32I, M, A (optional, off on the C3) and C; Zicsr; Zifencei (`fence.i` as a no-op)
- [x] Machine-mode CSRs: `mstatus`, `misa`, `mie`/`mip`, `mtvec` (direct and vectored), `mscratch`, `mepc`, `mcause`, `mtval`, `mcycle[h]`, `minstret[h]`, `mcountinhibit`, ID CSRs, user counter shadows
- [x] Exceptions (misaligned, access fault, illegal, breakpoint, ecall) and interrupts with ESP32-C3 line mask; `wfi`; `HartConfig::mie_mip` (off on the C3, which has no `mie`/`mip` CSRs and takes the interrupt controller's line directly)
- [x] Cycle model (`TimingModel`, uncalibrated C3 estimates) and `SimTime` mapping (`run_until`, `idle_until`, `local_time`)
- [x] Debugger API: `step`, `run`, breakpoints, halt/resume/reset, register/CSR access, `disassemble_at`
- [x] riscv-tests harness (`tests/riscv_tests.rs`) and build scripts (`scripts/build-riscv-tests.sh` / `.ps1`), CI job
- [x] Engine adapter: the `of-core` kernel drives the hart in quanta (done at SoC level as `of_soc_esp32c3::Esp32c3Engine`; register the CPU before the analog engine)
- [x] `of-soc-esp32c3`: memory map bus (IROM/DROM flash windows, IRAM/DRAM, ROM, RTC fast, peripherals) on top of `Bus` (`SocBus`; no cache MMU yet)
- [ ] ESP32-C3 custom CSRs (e.g. performance counters `mpcer`/`mpcmr`/`mpccr`, GPIO CSRs) through `Bus::csr_read`/`csr_write`
- [x] Interrupt matrix model feeding `set_interrupt_lines` (priority, threshold, edge/level; `of_soc_esp32c3::IntMatrix`)
- [ ] Calibrate `TimingModel` against a real ESP32-C3 (cycle counts of benchmark loops, flash cache misses)
- [ ] Decode cache (invalidated by `fence.i` and stores to code) for speed; benchmark MIPS
- [ ] Debug triggers (`tselect`/`tdata1`/`tdata2`) for hardware breakpoints and watchpoints; then pass `rv32mi-p-breakpoint`
- [ ] PMP (the C3 has 16 entries) and U-mode if firmware needs them; then pass `rv32mi-p-pmpaddr`
- [ ] `of-gdb`: remote serial protocol stub on top of the debugger API
- [ ] riscv-arch-test (the newer compliance suite) once a signature-dump harness exists

## P0 Foundations (weeks 0 to 6)

- [x] Cargo workspace with stub crates matching the planned layout
- [x] Licences (AGPL-3.0-or-later code, CC-BY-SA-4.0 parts and docs; relicensed from MIT OR Apache-2.0 / CC-BY-4.0 in November 2026), NOTICE, README, CONTRIBUTING, Code of Conduct
- [x] rustfmt, clippy, cargo-deny and toolchain configuration
- [x] CI workflow on GitHub Actions: fmt, clippy, cargo-deny, tests on Linux/Windows/macOS, wasm32 check, GUI check, riscv-tests and the test-firmware rebuild check
- [x] Example part (`parts/sensors/bosch-bme280/part.toml`) parsed by `of-parts`
- [x] Agent guides (`AGENTS.md`, `.grok/rules/`, this roadmap)
- [x] GUI scaffold ported to eframe 0.37 (`App::ui`) with X11/Wayland backends so the Linux CI GUI check builds
- [ ] `of-core`: scheduler snapshot/restore and replay log
- [x] `of-core`: `Engine` trait (`advance_to(limit, ctx) -> reached`, `next_event`, `on_event`) and `Kernel` (quantum slices, events, breakpoints, host stops); used by the analog engine and the ESP32-C3 engine
- [ ] Project file format v0 (`project.toml` + circuit file) with serde round-trip tests
- [ ] `of-parts`: JSON Schema generation (schemars) and a validator command in `of-cli`
- [ ] `of-cli`: `ohmforge validate-parts parts/`
- [ ] RFC template in `rfcs/0000-template.md`
- [ ] mdBook docs skeleton
- [x] Choose the final project name: Ohmforge
- [x] Fill in the Code of Conduct contact

## P1 Analog core (weeks 4 to 16)

Status: done. `of-analog` has 22 unit tests, 29 analytic tests, 3 engine tests, 1 doc test and an ngspice reference set of 39 circuits (80 signals: OP, DC, nested DC, AC magnitude/phase and transient, including the AMS1117 macromodel and the MVP board) all within 1 %; `of-netlist` has 8 tests, `of-parts` 4, `of-core` 6, and `of-components` 15 (parts bridge, AMS1117 datasheet checks, the MVP board from parts against ngspice, and a scripted-CPU co-simulation). Design notes: `AGENTS.md` section 3b.

- [x] SPICE netlist subset parser (`of-netlist::spice`): R C L V I E G F H D Q M, `.model`, `.subckt`/`X` flattening, `.param` constants, `.include`, `.options`, `.ic`, `.op`, `.dc`, `.tran`
- [x] Netlist builder (`of_analog::netlist::build`): deck to MNA unknowns, ground handling, voltage-source and inductor branches, clear errors and unsupported-parameter warnings
- [x] Sparse MNA with faer sparse LU: pattern built once, symbolic factorisation reused, refactorisation skipped when the matrix is unchanged (linear fast path)
- [x] R, C, L (with `ic=`), independent V/I sources (DC, PULSE, SIN, EXP, PWL) and linear controlled sources E/G/F/H
- [x] DC operating point: Newton-Raphson with pnjlim/fetlim/limvds, then damped Newton, gmin stepping and source stepping; DC sweeps with warm starts
- [x] Transient: trapezoidal (default) and variable-step Gear-2, charge-based LTE step control (SPICE `CKTterr`), exact breakpoints at source corners, `uic`
- [x] Diode: Shockley with emission coefficient, series R, depletion and diffusion charge, breakdown (`BV`/`IBV`)
- [x] BJT: Gummel-Poon subset (IS BF BR NF NR VAF VAR IKF IKR ISE NE ISC NC RB RC RE CJE VJE MJE CJC VJC MJC TF TR FC)
- [x] MOSFET level 1 (Shichman-Hodges) with body effect, channel-length modulation, RD/RS, bulk diodes and overlap capacitances
- [x] `ohmforge run netlist.cir [--out x.csv] [--vcd x.vcd]`
- [x] ngspice comparison suite (`tests/ngspice_compare.rs`, golden CSVs generated by `scripts/gen-ngspice-golden.sh`, ngspice used only as an external program): all within 1 % of span
- [x] MOSFET Meyer gate capacitances (TOX) and bulk junction capacitances (CBD/CBS, CJ/CJSW with AD/AS/PD/PS, MJ/MJSW/PB/FC, JS)
- [x] MOSFET level 3 (SPICE semi-empirical: ETA, THETA, KAPPA, VMAX, XJ, DELTA, NFS, NSUB-derived parameters), exact derivatives via dual numbers; EKV-lite not planned
- [x] BJT extras: XTF/VTF/ITF transit-time modulation, IRB/RBM, CJS substrate cap (4th node), XCJC split, excess phase (PTF, transient), temperature (TEMP/TNOM, XTI/EG/XTB); gmin on the leakage diodes only, as SPICE
- [x] Temperature for diodes (IS, junction potential and CJO, BV with TCV, NBV) and MOSFETs (KP/U0, PHI, VTO, IS/JS, bulk caps); `.temp`, `.options temp/tnom`, instance `temp`/`dtemp`
- [x] Coupled inductors (`K`), switches (`S`/`W`, SW/CSW models with hysteresis), LED model (`.model x led(vf if n rs wavelength eqe)` with `popt()` and `bright()` probes), capacitor ESR/ESL, inductor `rser`, `tc1`/`tc2` tempcos and stored `tol=`
- [x] `.ac` small-signal analysis (dec/oct/lin, complex MNA on the same sparse pattern, magnitude/phase output); nested `.dc`; `.param` expressions with functions; subcircuit parameters and subcircuit-local `.model`/`.param`
- [x] POLY(n) E/G sources (SPICE2 term order, exact Jacobian), used by the regulator thermal network
- [x] Bridge from `of-components` parts to `of-analog` devices (`of_components::analog`): `part.toml` `[[pin]]` and `[analog]` sections, pin mapping, typical/min/max/deviation corners from tolerances, placement overrides, subcircuit parts flattened per instance, SPICE export (`to_spice`); parts: resistor, MLCC, tantalum capacitor, red 0603 LED, small-signal NPN, AMS1117-3.3, BME280 supply load
- [x] Engine adapter for the `of-core` kernel (`of_analog::AnalogEngine`): sliced transient keeping integration history, parameter-change events, `PinDriver` GPIO outputs, threshold watches that post interrupts at the exact crossing, breakpoints and host stops
- [x] AMS1117-3.3 macromodel (`parts/regulators/ams1117/ams1117.lib`, standard primitives only): dropout, quiescent current, line/load regulation, current limit, ripple rejection, load-step response, junction temperature with thermal limit; checked against the datasheet and ngspice
- [x] MVP analog checks: LED with series resistor, transistor switch and AMS1117 (the `mvp_board` circuit, also built from parts) within 1 % of ngspice
- [ ] Switch timestep control near thresholds (ngspice `SWtrunc`), BJT excess phase in `.ac`, `.noise`/`.tf`/`.sens`, `.step`/Monte Carlo using `tol=`
- [ ] Latent-subcircuit bypass (skip converged, quiescent subcircuits) for speed
- [ ] AMS1117-ADJ variant; junction temperature feeding back into device temperatures (electro-thermal coupling)

## P2 ESP32-C3 MVP (weeks 12 to 28)

- [x] RV32IMC interpreter passing riscv-tests (see phase one; riscv-arch-test still to do)
- [ ] ELF loading (`object`, `gimli`), flash image layout, ROM from esp-rom-elfs
  - [x] ELF32 RISC-V reader in `of-firmware` (segments, symbols; own code, no `object` dependency yet) and direct ELF boot (`Esp32c3::load_elf`)
  - [x] ESP flash image parser and writer (`of_firmware::EspImage`: header, segments, checksum, optional SHA-256, own SHA-256) and boot from a parsed image (`Esp32c3::load_flash_image`, chip ID checked)
  - [ ] DWARF line info (`gimli`), partition table / flash layout, ROM boot from esp-rom-elfs, cache MMU
- [ ] C3 memory map, interrupt matrix, systimer, timer groups, UART
  - [x] Memory map (ROM, SRAM IRAM/DRAM alias, flash cache windows, RTC fast, peripheral dispatch with read-back for unmodelled registers)
  - [x] SYSTIMER: two units (load, work enable), three comparators in target and period mode, interrupts (52-bit wrap and stall-on-CPU-halt not modelled)
  - [~] UART0 TX capture; baud timing and RX to do
  - [x] Interrupt matrix and CPU interrupt controller (62 sources, 31 lines, priority, threshold, edge/level, clear), traps into vectored `mtvec`
  - [x] Timer groups TIMG0/1 (timer 0 with divider, auto-reload alarm, load), MWDT, RTC WDT and super watchdog (stages, feed, write protection, flash-boot protection; resets reboot the loaded image); timer-interrupt firmware `firmware/esp32c3-timer-irq` (`tests/timer_irq.rs`)
- [ ] GPIO/IOMUX with electrical pin model (drive strength, thresholds, pulls)
  - [x] GPIO out/enable (+W1TS/W1TC), GPIO matrix `OUT_SEL` 128 and inversion, IO MUX function and drive strength, Thevenin pin model (estimated resistances)
  - [ ] Inputs with thresholds and hysteresis, pulls, open drain, interrupts
- [ ] ADC with attenuation, nonlinearity and noise; LEDC PWM
- [ ] Mixed-signal sync: CPU stops on MMIO, analog advances to the exact time
  - [x] `Esp32c3Engine`: stops after each peripheral access, pin changes become kernel events at the exact hart time (blinky drives an LED + resistor, `tests/blinky_led.rs`)
  - [ ] Stop before input reads so GPIO/ADC reads see the analog value at that instant
- [ ] egui schematic editor (place, wire, probe), oscilloscope, logic analyser with UART decode, serial monitor
  - [x] Read-only desktop blink demo (`of-ui` `BlinkDemo`, `cargo run -p of-app --features gui`): bundled blinky co-simulated with R1 330 Ω and a red LED, the schematic LED lit from the simulated current, an LED current trace and UART0 output. Not the editor or the real oscilloscope (GPU waveform path, decision d2)
- [ ] BOM v1: CSV with MPN, supplier links, ZAR cost; generated build checklist
- [ ] MVP demo: blink, button interrupt, potentiometer to ADC, LED dimming, all from unmodified Arduino/ESP-IDF/esp-hal ELFs

### First milestone: blinky end to end

Each step lands with its own tests; firmware is ours and tiny, built with zig and committed (CI checks it rebuilds byte-identical). Steps 1 to 6 are on `main`.

1. [x] Memory map and bus (`SocBus`): region routing, aliases, read-only ROM/flash, fetch rules (unit tests)
2. [x] ELF reader (`of-firmware`) and direct boot (`Esp32c3::load_elf`): hand-built ELF, truncation and malformed-file tests, real zig-built ELF
3. [x] GPIO + minimal IO MUX/GPIO matrix output path (unit tests per register and alias)
4. [x] SYSTIMER unit 0 read path, UART0 TX capture (unit tests; banner check in the blinky test)
5. [x] Blinky firmware (`firmware/esp32c3-blinky`, RV32IMC asm, SYSTIMER-timed): edges every 250 µs without drift (`tests/blinky.rs`)
6. [x] `Esp32c3Engine` + `PinDriver` + parts-library LED and resistor under the kernel: LED current switches with a 500 µs period, about 4 mA on (`tests/blinky_led.rs`)
7. [ ] Same blinky from esp-hal (Rust, `riscv32imc-unknown-none-elf`) once the ROM boot path and cache MMU exist (watchdogs and the interrupt matrix are in)
8. [ ] Arduino-ESP32 / ESP-IDF blink through the real ROM and a flash image

### MVP definition (exit criteria for 0.1)

- [ ] Unmodified ESP32-C3 ELF (Arduino-ESP32, ESP-IDF or esp-hal) boots from the real ROM
- [ ] Blink, button with interrupt, potentiometer into ADC, LED dimming via LEDC behave as on hardware
- [x] RC filter, LED with series resistor, transistor switch, AMS1117 regulator within 1 % of ngspice
- [ ] Oscilloscope, logic analyser with UART decode, multimeter probe, serial monitor
- [ ] BOM CSV export with MPNs, supplier links and ZAR cost, plus build checklist
- [ ] Desktop builds on Windows, macOS, Linux; deterministic headless runs in CI

## P3 Breadth (weeks 28 to 42), exit 0.2: common hobby projects run

- [ ] Classic ESP32 (Xtensa LX6) through a QEMU sidecar process (`of-cpu-qemu-ipc`), kept out of process for licence and stability reasons
- [ ] Peripherals: I2C, SPI, RMT, DAC (C3 and classic ESP32)
- [ ] Breadboard view alongside the schematic editor
- [ ] Parts library to about 50 parts (sensors, displays, drivers, regulators), each with a `part.toml`, behaviour and tests
- [ ] Op-amp, comparator and regulator macromodels and `.subckt` import (LM358, TL431; AMS1117 done in P1)
- [ ] Electromechanical parts: DC motor with back-EMF and inertia, relay coil and contacts, servo
- [ ] KiCad schematic/netlist export (`of-kicad`) and XLSX BOM export
- [ ] Release 0.2

## P4 Own Xtensa core and web (weeks 40 to 60), exit 0.3: classic ESP32 in the browser

- [ ] Own Rust Xtensa LX6/LX7 core (`of-cpu-xtensa`) replacing the QEMU sidecar where possible
- [ ] Web app (`of-web`, wasm32) running the simulation in the browser
- [ ] Wi-Fi virtual radio (simulated network between boards and a host bridge)
- [ ] Sleep modes and a power profiler (current over time, battery life estimate)
- [ ] Energy parts: Li-ion/LiPo with state of charge and internal resistance, coin cell, USB 5 V source
- [ ] Release 0.3

## P5 Realism (weeks 56 to 72), exit 0.4: validation report published

- [ ] Monte Carlo runs over part tolerances
- [ ] Thermal model (self-heating, ambient, tempco; MOSFET Rds(on) against temperature)
- [ ] Fault injection (opens, shorts, brown-outs, noisy supplies)
- [ ] Hardware-in-the-loop lab: compare simulation with real boards
- [ ] Supplier price plugins for the BOM
- [ ] Publish the validation report (simulation against hardware and ngspice); release 0.4

## P6 1.0 (weeks 70 to 84)

- [ ] Stable component API and WASM SDK for third-party parts (`of-component-api`, `of-plugin-wasm`)
- [ ] ESP32-S3 and ESP32-C6 support
- [ ] Complete documentation (mdBook user guide, part-authoring guide, API docs)
- [ ] BLE (stretch goal)
- [ ] Release 1.0
