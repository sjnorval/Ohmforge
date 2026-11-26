# Roadmap and task list

Derived from `docs/DESIGN_AND_PLAN.md` section 14. Tick a box only when the work is verified, all checks in `AGENTS.md` pass and it is committed; `[~]` means partly done (what is missing is noted on the item). Work on a feature branch counts once it is pushed with green CI. Status as of 26 November 2026 (`main` includes work through PR #99). The first P1/P2 landings were #7 to #12: P1, the blinky milestone, interrupts, timers and watchdogs, the ESP image parser and the desktop blink demo; P2 is still in progress. The ROM boot work is on `main` (#20), and the esp-hal blinky too (#21). GPIO inputs are on `main` (#22). ADC and LEDC PWM are on `main` (#24). The desktop demo picker (#26) and live playback, bench supply and virtual meters (#27) are on `main`, as are faster playback (#28), honest bench I/O (#29), baud-timed UART0 RX (#30), scope zoom (#33), the schematic router with hops (#35 to #41), the EILIi load screen (#40, #42), the engine design (#43), CPU/solver speedups (#44), the docs alignment (#51), green and blue 0603 LEDs (#52), the remaining P0 leftovers (#53) and the PSU return and current-averaging fix (#45, closed by #54), and RFC 0001 for the E2 component API (#55). Later: the E2 engine core (#62), E3 ESP32-C3 component (#63), BOM v1 (#65), 1× realtime (#67), the Arduino/ESP-IDF demo design (#69) and ESP-IDF blink (#71, #60 D1), the signal-flow plan (#72; G1 to G3 tracked in #74), the E4 circuit files (#73, #77 to #80; E4 is complete), instruments E5a to E5g (#49, through #98; listed follow-ups remain) and the NE555 macromodel (#99). The parts the library should grow to, and what each one needs from the simulator, are in the [common components checklist](COMPONENTS.md). The move to a base engine with plug-in components (the ESP32-C3, the bench PSU and every part as components on one engine API) is planned in [`design/engine-components.md`](design/engine-components.md) (phases E1 to E3 in P2, done; E4 done and E5 nearly done in P3, E6 still open; remaining issue #50).

**Current phase: P2 ESP32-C3 MVP, in progress** (ESP32-C3 SoC on the RISC-V core, co-simulated with the analog solver through the `of-core` kernel). P1 Analog core and phase one (the RISC-V CPU core, pulled forward from P2) are done; the first P2 slice (blinky end to end), the interrupt matrix, SYSTIMER alarms, timer groups, watchdogs, the ESP image parser, the desktop demo picker, live playback, the bench supply and virtual meters are on `main`. P0 leftovers continue alongside. 1× realtime (#61), BOM v1 (#59) and the ESP-IDF blink (#60 D1) are done. Next in P2: the remaining Arduino/ESP-IDF demos (#60, D2 onward). In P3, E4 is complete and E5a to E5g are on `main` (follow-ups on E5f and E5g remain); E6 is still open. The part creator and model/waveform interop plans are in [`design/part-creator.md`](design/part-creator.md) and [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md).

## Phase one: RISC-V CPU core (`of-cpu-riscv`)

Status: done; the core is implemented and tested (68 unit tests, 1 doc test, 75 of 77 riscv-tests passing, 2 expected-unsupported). Design notes: `AGENTS.md` section 3a.

- [x] `Bus` trait (sizes, access kinds, wait states, custom CSR hooks, sync requests) and a `Ram` device
- [x] RV32I decoder and disassembler; RVC expander (all RV32C forms, reserved encodings rejected)
- [x] Execution of RV32I, M, A (optional, off on the C3) and C; Zicsr; Zifencei (`fence.i` as a no-op)
- [x] Machine-mode CSRs: `mstatus`, `misa`, `mie`/`mip`, `mtvec` (direct and vectored), `mscratch`, `mepc`, `mcause`, `mtval`, `mcycle[h]`, `minstret[h]`, `mcountinhibit`, ID CSRs, user counter shadows
- [x] Exceptions (misaligned, access fault, illegal, breakpoint, ecall) and interrupts with ESP32-C3 line mask; `wfi`; `HartConfig::mie_mip` (off on the C3, which has no `mie`/`mip` CSRs and takes the interrupt controller's line directly)
- [x] Cycle model (`TimingModel`, uncalibrated C3 estimates) and `SimTime` mapping (`run_until`, `idle_until`, `local_time`)
- [x] Debugger API: `step`, `run`, breakpoints, halt/resume/reset, register/CSR access, `disassemble_at`
- [x] riscv-tests harness (`tests/riscv_tests.rs`) and build scripts (`scripts/build-riscv-tests.sh` / `.ps1`), CI job
- [x] Engine adapter: the `of-core` kernel drives the hart in quanta (done at SoC level as `of_soc_esp32c3::Esp32c3Engine`; register the CPU before the analog engine)
- [x] `of-soc-esp32c3`: memory map bus (IROM/DROM flash windows, IRAM/DRAM, ROM, RTC fast, peripherals) on top of `Bus` (`SocBus`; cache MMU)
- [~] ESP32-C3 custom CSRs through `Bus::csr_read`/`csr_write`: performance counters `mpcer`/`mpcmr`/`mpccr` and `upcer`/`upcmr`/`upccr` (cycle event only) and the user trap CSRs (stored only); one counter behind both names with `mpcmr` reset to count and saturate, PMP and debug trigger CSRs (stored, not enforced) (#21); other events and the GPIO CSRs to do
- [x] Interrupt matrix model feeding `set_interrupt_lines` (priority, threshold, edge/level; `of_soc_esp32c3::IntMatrix`)
- [ ] Calibrate `TimingModel` against a real ESP32-C3 (cycle counts of benchmark loops, flash cache misses)
  - [~] Decode cache (invalidated by `fence.i`, RISC-V I-stream coherence): a 4096-line direct-mapped cache in the hart that skips the bus fetch on hits; monomorphised `Bus` run loop and quiescent-slice GPIO batching (#28, #44); ALU hot path and empty-breakpoint `run_tight` (#61). Host throughput on busy demos is still ~0.17–0.22× realtime (see live-playback note); 1× needs a faster execution strategy (basic-block / JIT), not more interpreter micro-opts
- [ ] Debug triggers (`tselect`/`tdata1`/`tdata2`) for hardware breakpoints and watchpoints; then pass `rv32mi-p-breakpoint`
- [ ] PMP (the C3 has 16 entries) and U-mode if firmware needs them; then pass `rv32mi-p-pmpaddr`
- [ ] `of-gdb`: remote serial protocol stub on top of the debugger API (scheduled in P3 as editor-integration item I3, [`design/editor-integration.md`](design/editor-integration.md))
- [ ] riscv-arch-test (the newer compliance suite) once a signature-dump harness exists

## P0 Foundations (weeks 0 to 6)

- [x] Cargo workspace with stub crates matching the planned layout
- [x] Licences (AGPL-3.0-or-later code, CC-BY-SA-4.0 parts and docs; relicensed from MIT OR Apache-2.0 / CC-BY-4.0 in November 2026), NOTICE, README, CONTRIBUTING, Code of Conduct
- [x] rustfmt, clippy, cargo-deny and toolchain configuration
- [x] CI workflow on GitHub Actions: fmt, clippy, cargo-deny, tests on Linux/Windows/macOS, wasm32 check, GUI check, riscv-tests and the test-firmware rebuild check
- [x] Example part (`parts/sensors/bosch-bme280/part.toml`) parsed by `of-parts`
- [x] Agent guides (`AGENTS.md`, `.grok/rules/`, this roadmap)
- [x] GUI scaffold ported to eframe 0.37 (`App::ui`) with X11/Wayland backends so the Linux CI GUI check builds
- [ ] `of-core`: scheduler snapshot/restore and replay log (still open; not required for the P0 exit of “repo public, CI green”)
- [x] `of-core`: `Engine` trait (`advance_to(limit, ctx) -> reached`, `next_event`, `on_event`) and `Kernel` (quantum slices, events, breakpoints, host stops); used by the analog engine and the ESP32-C3 engine
- [ ] Project file format v0 (`project.toml` + circuit file) with serde round-trip tests (deferred: circuit file is engine phase E4 / #48, [`design/engine-components.md`](design/engine-components.md); `project.toml` can follow on top)
- [ ] `of-parts`: JSON Schema generation via schemars (still open; `validate-parts` covers live checks without exporting a schema file yet)
- [x] `of-cli`: `ohmforge validate-parts parts/` (parses every `part.toml`, runs `PartFile::check`, requires CC-BY-SA-4.0 licence fields)
- [x] RFC template in `rfcs/0000-template.md` (#16)
- [x] mdBook docs skeleton (`docs/book.toml`, `docs/src/`; full user guide remains P6)
- [x] Choose the final project name: Ohmforge
- [x] Fill in the Code of Conduct contact
- [x] Unused-dependency warnings cleared in stub crates and `of-cli` (#14, #15); planned edges stay in the `AGENTS.md` crate map and return to `[workspace.dependencies]` when a consumer lands


## P1 Analog core (weeks 4 to 16)

Status: done. `of-analog` has 26 unit tests, 29 analytic tests, 5 engine tests, 1 doc test and an ngspice reference set of 39 circuits (80 signals: OP, DC, nested DC, AC magnitude/phase and transient, including the AMS1117 macromodel and the MVP board) all within 1 %; `of-netlist` has 8 tests, `of-parts` 4, `of-core` 7, and `of-components` 21 (parts bridge, AMS1117 datasheet checks, the MVP board from parts against ngspice, and a scripted-CPU co-simulation). Design notes: `AGENTS.md` section 3b.

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
  - [ ] DWARF line info (`gimli`)
  - [x] Partition table / flash layout: `of_firmware::PartitionTable` (parse and write, MD5 row, boot-app choice without OTA data) and 64 KiB-aligned app images (`EspImage::align_flash_segments`)
  - [x] SPI flash and cache MMU: 4 MiB `SpiFlash` behind SPI0/SPI1 (`CMD` operations and user transactions), EXTMEM and the 128-entry MMU shared by IROM and DROM (no cache lines or miss timing)
  - [x] ROM boot from esp-rom-elfs: the real revision 3 ROM runs (`Esp32c3::power_on_rom`; fetched by `scripts/fetch-esp-rom-elfs.*`, pinned and checksummed, never committed): banner and reset cause, blank flash, direct boot of `firmware/esp32c3-xip-blinky`, loading and SHA-256-checking an image at flash 0, watchdog resets back into the ROM, download strapping (`tests/rom_boot.rs`); ROM-lite fast boot (`Esp32c3::boot_flash`: direct boot, partition-table app, image at 0; `tests/flash_boot.rs`)
  - [x] ESP-IDF second-stage bootloader through the real ROM (#21): espflash 4.7.0's prebuilt bootloader (ESP-IDF v6.1; fetched with the ROM, pinned and checksummed, never committed) reads the partition table, verifies and maps the app and boots it (`tests/esp_hal_blinky.rs`). Needed: misaligned loads and stores on the C3 core, the performance counter enabled at reset, the RNG, PMP CSRs, the SYSTEM CPU clock (XTAL or PLL, 20 to 160 MHz) and TIMG RTC calibration
- [ ] C3 memory map, interrupt matrix, systimer, timer groups, UART
  - [x] Memory map (ROM, SRAM IRAM/DRAM alias, flash cache windows, RTC fast, peripheral dispatch with read-back for unmodelled registers)
  - [x] SYSTIMER: two units (load, work enable), three comparators in target and period mode, interrupts (52-bit wrap and stall-on-CPU-halt not modelled)
  - [x] UART0 TX capture, configuration registers with reset values and the configured baud rate (#21); RX FIFO (`push_uart_rx`); baud-timed RX (one frame per byte at the configured baud) and RX interrupts (`RXFIFO_FULL`, `RXFIFO_TOUT`, `RXFIFO_OVF`, `INT_RAW`/`INT_ST`/`INT_ENA`/`INT_CLR`) (#30). The SoC TX log is capped at 64 KiB
  - [x] Interrupt matrix and CPU interrupt controller (62 sources, 31 lines, priority, threshold, edge/level, clear), traps into vectored `mtvec`
  - [x] Timer groups TIMG0/1 (timer 0 with divider, auto-reload alarm, load), MWDT, RTC WDT and super watchdog (stages, feed, write protection, flash-boot protection; resets reboot the loaded image); timer-interrupt firmware `firmware/esp32c3-timer-irq` (`tests/timer_irq.rs`)
- [ ] GPIO/IOMUX with electrical pin model (drive strength, thresholds, pulls)
  - [x] GPIO out/enable (+W1TS/W1TC), GPIO matrix `OUT_SEL` 128 and inversion, IO MUX function and drive strength, Thevenin pin model (estimated resistances)
  - [x] Inputs (#22): `GPIO_IN` from circuit node voltages through V_IL/V_IH watches (0.25/0.75 VDD, the datasheet limits, acting as hysteresis), `FUN_IE`, weak pull-up/pull-down (45 kΩ) in the analog pin model, datasheet reset pad settings, open drain (#21), pin interrupts (`INT_TYPE` edge and level, `INT_ENA`, `STATUS` W1TS/W1TC, `PCPU_INT`/`PCPU_NMI_INT`, matrix sources 16 and 17), the input-select matrix (`Gpio::peripheral_input`); esp-hal button firmware, polled and on an interrupt, co-simulated with a push-button part including bounce (`tests/esp_hal_button.rs`). V_IH/V_IL watches are relative to the live `vdd` node (a Vset change does not need Reset). The live bench SW1 uses the same bounce train (`of_components::bounce`), on by default, toggleable and seeded
  - [ ] Input synchronisers and glitch filter, GPIO wake-up, IO MUX direct peripheral inputs
- [ ] ADC with attenuation, nonlinearity and noise; LEDC PWM
  - [x] SAR ADC one-shot conversions (on `main`, #24): ADC1 channels 0 to 4 and ADC2 channel 0, four attenuations, 12-bit codes, conversion time from the SAR clock (25 cycles), done interrupts (source 43); each conversion samples the circuit node at its start (`Esp32c3Engine::link_adc`, `AnalogEngine::sample_on`); codes are the inverse of Espressif's line-plus-curve calibration with the chip's eFuse values (Espressif's documented example chip), so esp-hal's `AdcCalCurve` reads the input back within 3 mV
  - [ ] ADC nonlinearity and noise beyond the calibration curve, input sampling-capacitor load, ADC2 errata, DMA/pattern-table (continuous) mode, filters and thresholds, temperature sensor
  - [x] LEDC (on `main`, #24): four timers (APB, RC_FAST, XTAL clocks; fractional divider; 1- to 14-bit resolution; pause, reset, `PARA_UP` at overflow), six channels (Hpoint, duty with dither, hardware fades, overflow counter, idle level), interrupts (source 23), GPIO matrix output signals 45 to 50; each PWM edge reaches the circuit at its exact counter tick (no averaging)
  - [x] Potentiometer part (`potentiometer` analog kind, `bourns.ptv09a-b103`, turned at run time with `BuiltBoard::pot_params`); esp-hal pot dimmer and hardware LED fade (`firmware/esp32c3-esp-hal-adc-pwm`, `tests/esp_hal_adc_pwm.rs`); desktop slider and averaged LED in `of-ui`
  - [x] Green and blue 0603 LED parts (`generic.led-green-0603`, `generic.led-blue-0603`; #52, closes #17), same Shockley/`popt` model as the red 0603 part, with datasheet Vf checks in the bridge tests
- [ ] Mixed-signal sync: CPU stops on MMIO, analog advances to the exact time
  - [x] `Esp32c3Engine`: stops after each peripheral access, pin changes become kernel events at the exact hart time (blinky drives an LED + resistor, `tests/blinky_led.rs`)
  - [x] Stop before input reads so GPIO/ADC reads see the analog value at that instant: GPIO (#22: `Bus::defer_load`, lazy `wfi` woken at the input event; a busy core may take a pin interrupt up to one slice late, bounded by `Esp32c3Engine::set_max_lead`); ADC (#24: the engine stops at the conversion start and the analog engine answers with the node voltage at that instant)
- [ ] Base engine and plug-in components (design: [`design/engine-components.md`](design/engine-components.md)). The engine is the real world (nets, analog and digital solvers, time, events, measurement); every real part, the ESP32-C3 and the bench PSU become components on one API, and demos become circuit files. E1 to E3 are in P2; E4 to E6 are in P3, because release 0.1 does not wait for them. The editor's waypoint and re-edit items build on E4 and E6 and the component-based BOM on E6.
  - [x] E1 Design doc, roadmap, plan pointer and README note (`docs/design/engine-components.md` on `main`, #43)
  - [x] Bench PSU return path and current averaging (#45, closed by #54; landed before E2): the PSU minus terminal drawn and routed to GND on the blink, dimmer, fade and button boards, a check that every supply has both connections, a test that the current out of `+` equals the current back into `−`, and LED and PSU current averaged over whole PWM periods (no readout chatter at constant duty)
  - [x] E2 Engine core (`of-engine`) and the `Component` trait (#46): elaboration into `of-analog` devices, registry, invariants (one ground, every source has a return path, KCL, no floating nets), terminal currents; port the resistor, pot, LED, push button and bench PSU (`+` and `−` terminals). RFC Accepted ([`rfcs/0001-component-api.md`](../rfcs/0001-component-api.md)); `of-engine` + `LibraryComponent` / `BenchPsu` landed with parity and KCL tests — demos still assemble in `of-ui` until E3/E4
  - [x] E3 ESP32-C3 as a component (#47, #63): `Esp32c3Component` + `espressif.esp32-c3` with GPIO half-bridges on its own 3V3/GND, quiescent load, ADC/input hooks, UART0 port name, `take_engine`, pad helpers moved out of `of-ui`; engine blinky parity test passes. Desktop demos still use the thin `of-ui` pin wrappers; re-expressing the GPIO input / ADC / PWM demos on the engine moves to E4b (#48). Needs E2
  - E4 to E6 (circuit files, instruments, parts library and BOM): moved to P3 / 0.2, see below
- [ ] egui schematic editor (place, wire, probe), oscilloscope, logic analyser with UART decode, serial monitor
  - [x] Read-only desktop blink demo (`of-ui` `BlinkDemo`, `cargo run -p of-app --features gui`): bundled blinky co-simulated with R1 330 Ω and a red LED, the schematic LED lit from the simulated current, an LED current trace and UART0 output. Not the editor or the real oscilloscope (GPU waveform path, decision d2)
  - [x] Live desktop playback (worker thread, realtime speed control, live pot/button, scrolling scope, clickable SW1, tooltips, configurable bench PSU on the VDD rail, virtual voltmeter/ammeter) on `main` (#27). Window-tied scope retention, 1-2-5 time/div zoom (10 µs/div to 1 s/div) and playback down to 1e-6× are on `main` (#33). The analog solver holds a settled DC solution instead of re-solving every slice, and the hart caches decoded instructions (#28). Further CPU/solver speedups (decode-cache hits skip fetch, monomorphised bus, quiescent GPIO batching; #44; ALU / `run_tight` path in #61) leave release realtime on a typical desktop at about **0.21–0.22× dimmer**, **0.17× blink / fade / button-poll**, and far above realtime for button-irq (`wfi`). **1× on the busy demos is not reachable with the instruction interpreter alone** (#61): button-poll does ~11.6 M retired instructions per 100 ms sim with zero analog steps (~20 MIPS host) while 160 MHz at ~0.7 IPC needs ~6× that throughput; next step is a basic-block cache or host JIT that preserves exact PWM/GPIO timing and determinism. Release tests keep a 0.15× dimmer floor so #44 does not regress. GPIO thresholds track live Vset, and clickable SW1 bounces with the firmware-test train (#29).
  - [~] Serial monitor: lean console in `of-instruments` / `of-ui` (TX log, Send delivers to UART0 RX at the configured baud when attached; hex view and find, #30). Soft-front-panel hosting still to do (d7)
  - [ ] Schematic wire routing (design: [`docs/design/wire-routing.md`](design/wire-routing.md); keeps the existing schematic look). The P3 breadboard view and KiCad export consume the same paths.
    - [x] Data model and blink fixtures (no visual change); golden geometry tests
    - [x] Orthogonal grid router (A*/Lee, stubs, T-junctions, body clearance); blink uses generated paths
    - [x] Port the pot-dimmer and push-button demos
    - [x] Crossing policy (avoid, then hop) and rip-up and reroute
    - [ ] Optional 45° PCB-style mode (off by default)
    - [ ] User-editable waypoints and click-to-wire (editor slice; no editor chrome in the earlier PRs; needs the E4 circuit files and E6 symbols, so it follows them in P3)
    - [ ] Interactive trace re-edit mode in the schematic editor: manually re-route or adjust auto-generated traces (drag segments; move, add or remove waypoints/bends; re-run autoroute on a net); edits keep the same invariants (no body crossings, no foreign-net overlap, T-junctions)
- [x] BOM v1: CSV with MPN, supplier links, ZAR cost; generated build checklist (for 0.1 from the demo boards' placements via `of-bom` / `ohmforge bom`; moves onto components with E6, #50, in P3) (#59)
- [ ] MVP demo: blink, button interrupt, potentiometer to ADC, LED dimming, all from unmodified Arduino/ESP-IDF/esp-hal ELFs (all four run from esp-hal: blink, the button polled and on an interrupt (#22), and the pot dimmer and LED fade (#24); the desktop app can play the blink, dimmer, fade and push-button demos; Arduino and ESP-IDF builds still to do, design [#60](https://github.com/sjnorval/Ohmforge/issues/60) / [`design/arduino-idf-mvp-demos.md`](design/arduino-idf-mvp-demos.md))

### First milestone: blinky end to end

Each step lands with its own tests; firmware is ours and tiny, built with zig and committed (CI checks it rebuilds byte-identical). Steps 1 to 7 are on `main`.

1. [x] Memory map and bus (`SocBus`): region routing, aliases, read-only ROM/flash, fetch rules (unit tests)
2. [x] ELF reader (`of-firmware`) and direct boot (`Esp32c3::load_elf`): hand-built ELF, truncation and malformed-file tests, real zig-built ELF
3. [x] GPIO + minimal IO MUX/GPIO matrix output path (unit tests per register and alias)
4. [x] SYSTIMER unit 0 read path, UART0 TX capture (unit tests; banner check in the blinky test)
5. [x] Blinky firmware (`firmware/esp32c3-blinky`, RV32IMC asm, SYSTIMER-timed): edges every 250 µs without drift (`tests/blinky.rs`)
6. [x] `Esp32c3Engine` + `PinDriver` + parts-library LED and resistor under the kernel: LED current switches with a 500 µs period, about 4 mA on (`tests/blinky_led.rs`)
7. [x] Same blinky from esp-hal (Rust, `riscv32imc-unknown-none-elf`, #21): `firmware/esp32c3-esp-hal-blinky` (esp-hal 1.2.2, unmodified `esp_hal::init` with `CpuClock::max()`, 500 µs half period) boots through the real ROM and the ESP-IDF bootloader, prints its banner at 160 MHz and drives the LED circuit (`tests/esp_hal_blinky.rs`); CI rebuilds it byte for byte
8. [~] Arduino-ESP32 / ESP-IDF blink through the real ROM and a flash image (design: [`design/arduino-idf-mvp-demos.md`](design/arduino-idf-mvp-demos.md), #60; **ESP-IDF blink done**: `firmware/esp32c3-idf-blinky`, `tests/idf_blinky.rs`, ROM `.rodata.interface` + `SYSTEM_CPU_INTR_FROM_CPU_*` for FreeRTOS yield; Arduino-ESP32 blink still to do)

### MVP definition (exit criteria for 0.1)

- [ ] Unmodified ESP32-C3 ELF (Arduino-ESP32, ESP-IDF or esp-hal) boots from the real ROM (Arduino/IDF side: [`design/arduino-idf-mvp-demos.md`](design/arduino-idf-mvp-demos.md), #60)
- [ ] Blink, button with interrupt, potentiometer into ADC, LED dimming via LEDC behave as on hardware (esp-hal done; Arduino/IDF: same design note)
- [x] RC filter, LED with series resistor, transistor switch, AMS1117 regulator within 1 % of ngspice
- [~] Oscilloscope, logic analyser with UART decode, multimeter probe, serial monitor (E5a to E5g are on `main`: meters, scope, PSU face, logic analyser, UART console and `ohmforge run --log` / `--uart`; remaining: E5f resistance mode and mid-run meter state, E5g finer desktop capture and a demo that lists a console or analyser, plus d7 soft-front-panel hosting)
  - [x] Virtual voltmeter/ammeter on the live schematic (click two probe points; non-disturbing ammeter on a unique branch) on `main` (#27); E5c reads any circuit through the measurement service
  - [~] Serial monitor: lean console (TX log, Send delivers to UART0 RX at the configured baud when attached; hex view and find, #30). E5g adds the UART console instrument and `--uart`. d5 shell hosting still to do (d7)
- [x] BOM CSV export with MPNs, supplier links and ZAR cost, plus build checklist (#59)
- [x] EILIi startup animation as the desktop app load screen (packed 24 fps JPEG frames, no video decoder; click/Esc/any key skips; persistent "Show startup animation" setting; `--no-splash` / `OF_NO_SPLASH` still override).
- [ ] Desktop builds on Windows, macOS, Linux; deterministic headless runs in CI

## P3 Breadth (weeks 28 to 42), exit 0.2: common hobby projects run

- [~] Base engine phases E4 to E6 (moved from P2 on 23 November 2026; design: [`design/engine-components.md`](design/engine-components.md); E1 to E3 are in P2). E4 is complete; E5a to E5g are on `main` with listed follow-ups; E6 remains. The schematic editor's waypoint and re-edit work and the component-based BOM build on these.
  - [x] E4 (#48) Circuit files (serde, round-trip): blink, dimmer, fade and button drive both the simulation and the shared router, with output parity (firmware traces and wiring goldens match or have reviewed updates). Needs E3. Complete (#73, #77 to #80).
    - [x] E4a Format ([RFC 0003](../rfcs/0003-circuit-file.md)): `of-circuit` parses `*.circuit.toml` with line and column errors, validates refs, nets, ground, pins against the parts library and firmware SHA-256, and saves a canonical form that round-trips byte for byte; `ohmforge circuit check` and `circuit fmt [--check] [--update-hashes]`; `circuits/LICENSE.md` (CC-BY-SA-4.0). On `main` (#73)
    - [x] E4b Engine session and demo files: model registry and runtime in `of-engine`, `bench.psu` part, `blink`, `dimmer`, `fade`, `button` and `button-irq` in `circuits/demos/` with trace parity against the legacy demos, `ohmforge run <file.circuit.toml>`. E4b-1 done in #77: legacy trace goldens, component API addendum, `Registry`, run-time pot, switch, bounce and PSU CV/CC, ESP32-C3 boot paths and params, `Session`, and parity of the registry-built demos on `Session`. The accepted parity exception is in RFC 0003 (1 % on LEDC-edge samples against the cross-platform goldens; the dimmer's 3.5 nA PSU exception went in E4c). E4b-2 done in #78: the `of-circuit` builder (`build`) with `EmbeddedResolver`, the embedded demos and the fetched-ROM lookup in `FsResolver`; the five demo files with SHA-256 and the ESP32-C3 idle draw (5 mA then; the datasheet's 23 / 16 mA since E4c); the `blink.idle-draw` golden; parity from the files; `ohmforge run`; CI `circuit fmt --check` and `circuit check` on `circuits`; refs, schematic ids and pin names checked against the hand-written boards
    - [x] E4c Schematic from circuit files (wiring goldens byte-identical), desktop demos behind a flag. Done in #79: `wiring::from_circuit` (layout from the file, schematic equal to the hand-written board for every demo and pot position, routes byte-identical to the goldens); `SessionSim` runs the desktop demos on `Session` from the embedded files and parts (`of_circuit::demos::parts`), with readings matching the hand-wired demos; `of-app --circuit-files` (also `OF_CIRCUIT_FILES` and the `circuit_files` preference); `Session::terminal_currents` and the switch `bounce_seed` param; parity runs the desktop `SessionSim`; the ESP32-C3 supply current from the datasheet (23 mA running, 16 mA in `wfi`, at 160 MHz) and the dimmer's GPIO2 pad on both paths, so the readings match with no per-demo exception
    - [x] E4d Switch-over: the files become the only path, the flag and the hand-wired boards go. Done in #80: the desktop always runs `SessionSim` from the embedded files; `--circuit-files`, `OF_CIRCUIT_FILES` and the `circuit_files` preference are removed (an old key is ignored); the hand-wired boards (`wiring/{blink,button,dimmer}.rs`), simulations (`Blink`, `PwmSim`, `ButtonSim`), pin and PSU helpers and `parity/legacy.rs` are deleted. The trace goldens are blessed from the circuit files (unchanged byte for byte); parity compares the registry-built `Session` with the files at the strict tolerance; the wiring and board tests check the route goldens
  - [~] E5 (#49) Instruments as components: measurement service in `of-instruments` (trace store, period-aligned averages, duty/frequency), voltmeter, ammeter, scope, logger and UART console on any circuit. Needs E2 (E4 for the desktop panels). Design: [RFC 0004](../rfcs/0004-instruments.md) (Accepted 2026-11-25, with verified DS1054Z and DP832 figures). E5a to E5g are on `main`; remaining follow-ups are listed on E5f and E5g.
    - [x] E5a Service core: `Signal` names, `Probeable`, `MeasurementService` with `SamplePlan`; `TraceBuf`, `SampleClock` and the averaging, duty/frequency maths moved from `of-ui` unchanged (re-exported), plus min/max and RMS; trace goldens unchanged
    - [x] E5b `Session` as `Probeable`; `SessionSim` samples the files' probes through the service. The LED scope reads the file's `LED current` probe (`i(R…)` from `terminal_currents`), bit-identical to the old V/R; the PSU ring moves in E5d
    - [x] E5c Ideal multimeter on any branch (`of_instruments::meter`); `CircuitReadout` glue removed. Decided (Stefan): diode current from the solver (`i(D1)`, within 1e-16 A of the series R), voltmeter and ammeter average whole PWM periods like a DMM's integrating ADC, a "through" chooser when several parts join two points (strict rule: U1 is offered wherever it has pins on both nets), resistance mode in E5f. On all five demos every meter the old readout resolved shows the same text and value within max(1 nA, 1 ppm), PSU included (RFC 0004 table; pinned by `the_meters_read_the_recorded_values_on_all_demos`)
    - [x] E5d PSU readout and scope on the service: the service owns the sample grid (the desktop loops step to it, grid identical sample for sample), `PsuReadout` builds the PSU face from `r(PSU.*)` recorded after each commit, `Scope` shows a channel raw or period-averaged, and the desktop supply types are `of_components::psu`'s. PSU face and LED average bit-identical on all five demos and in CC; goldens and pinned meters unchanged
    - [x] E5e `[[instrument]]` table (optional, `format` stays 1; ref, kind, profile, mode, probes, at, params, canonical save) and `ohmforge run --log` CSV/VCD of the probes and instruments (ideal, on the `--step` grid, through `Probeable`); `circuit check` parses and resolves every probe and instrument signal (case-sensitive names, two-pin `i(REF)`, instrument kind versus signal) with line and column; the five demos carry an ideal `XMM1` (DC A) and `XSC1` on `LED current`. Digital `d(..)` logging waits for E5g, real profiles for E5f
    - [x] E5f Real-world profiles (Fluke 87V, Rigol DS1054Z, Rigol DP832; Saleae Logic 8 moves to E5g) and load elements: cited figures in `parts/instruments/<vendor>.<model>/instrument.toml`, `of_instruments::profile` (autorange, `OL`, ± bands, scope quantisation, PSU readback), `of_circuit::loads` (a real instrument re-elaborates the circuit with its input; series ammeter burden) and the desktop Profiles row for the meters, scope and PSU readout. Follow-ups: resistance mode (test current) and keeping time and state when a meter is added mid-run
    - [x] E5g Logic analyser and UART console instruments: `d(REF.pin)` through a logic input (ideal 1.65 V; real V_IL/V_IH with hold), the Saleae Logic 8 profile and `LogicAnalyser`, `d(..)` VCD wires in `ohmforge run --log`, the 8N1 `UartDecoder` behind `ohmforge run --uart`, `circuit check` rules for `uart` and `logic`, the ESP32-C3 driving GPIO21 (U0TXD) with UART0's frames when it is wired (decoded text matches the chip's serial output end to end), and desktop console panels and a logic analyser view. The first demo to list a console and an analyser is `esp32c3-555-clock`. Follow-ups: desktop capture finer than 1 µs, and TX FIFO throttling
  - [ ] E6 (#50) Parts library and BOM from components: `[symbol]`, pin kinds and `bom` in `part.toml`; the BOM comes from the circuit's components. Needs E4
- [ ] Classic ESP32 (Xtensa LX6) through a QEMU sidecar process (`of-cpu-qemu-ipc`), kept out of process for licence and stability reasons
- [ ] Peripherals: I2C, SPI, RMT, DAC (C3 and classic ESP32); the bus part is designed in [`design/buses.md`](design/buses.md), with TWAI/CAN in P4
  - [ ] B1 Bus ports and device framework: typed I2C, SPI, UART and CAN ports in the component API (RFC 0002), transaction-level and pin-level buses with one protocol model per device. Needs E2, E3, E4
  - [ ] B2 I2C: C3 controller (master, then slave), open-drain pads, pull-ups and rise-time checks against the NXP UM10204 limits, BME280 I2C behaviour, MPU-6050 (GY-521) part. Needs B1
  - [ ] B3 SPI: GPSPI2 master (modes 0 to 3, CS, DMA), then slave; SD card (SPI mode) and ST7735 models. Needs B1
  - [ ] B4 UART complete: timed TX, UART1, data/parity/stop options, RTS/CTS and XON/XOFF, framing and parity errors at pin level, RS-485 direction; feeds the I2 virtual serial port. Needs B1
  - [ ] B5 Logic analyser decoders (UART, I2C, SPI) and export to VCD and sigrok `.sr` (extends the P2 logic analyser item and UX decision d4). Needs E5, B2 to B4
- [ ] Flash like real hardware: expose the simulated chip's UART0 (and later the C3's USB Serial/JTAG) as a virtual serial port (TCP/RFC 2217 first, then a host COM/PTY bridge), with the board's DTR/RTS auto-reset circuit driving EN and GPIO9 into the ROM's download mode, so unmodified `esptool`, `espflash`, the Arduino IDE and `idf.py flash` write the simulated SPI flash through the real ROM loader; flash contents persist with the project (editor-integration item I2, [`design/editor-integration.md`](design/editor-integration.md))
- [ ] Editor integration foundations (design: [`design/editor-integration.md`](design/editor-integration.md); the plugins follow in P4)
  - [ ] I1 Control API and headless mode: `ohmforge serve` and the same server in `of-app`, JSON-RPC 2.0 over a named pipe / Unix socket or loopback TCP with a session token; open project, load and flash firmware, reset, run/pause, UART and readings; `ohmforge flash` client. Needs E3 (E4 for `project.open`)
  - [ ] I2 Virtual serial port and auto-reset: the “Flash like real hardware” item above
  - [ ] I3 `of-gdb` GDB remote stub (from the phase-one list): breakpoints, registers and memory through the bus, virtual time stopped while halted
  - [ ] I4 Build-folder watch and hot reload (`DESIGN_AND_PLAN.md` §8 step 5): new firmware with the circuit state kept
- [ ] Breadboard view alongside the schematic editor (design: [`design/breadboard.md`](design/breadboard.md); uses the same circuit file as the schematic, true 2.54 mm geometry, jumper and fit checks; schematic routing stays in [`docs/design/wire-routing.md`](design/wire-routing.md)). Needs E4, E6, W1
- [ ] Official example gallery (design: [`design/examples-and-sharing.md`](design/examples-and-sharing.md)): at least five complete, documented, openable projects in the desktop app, copied into a user folder; CI smoke for each example. Needs E4, E6
- [ ] File-based project packs and share links (same design note): export and open a project folder or archive (`ohmforge pack` / `ohmforge unpack`) with no account; documented share-link form for the P4 web app; secrets never packed. Needs E4
- [ ] Compatibility and electrical-rule checks, first slice (design: [`design/compatibility-checks.md`](design/compatibility-checks.md)): cited interface and rating fields in `part.toml`, a visible Checks panel, and CLI findings for LED-without-resistor, supply out of range, logic-level mismatch and missing decoupling when the citations exist. The full `ohmforge review` rule set remains AI4 in P4. Needs E4, E6
- [ ] Parts library to about 50 parts (sensors, displays, drivers, regulators), each with a `part.toml`, behaviour, tests and, where a check uses a rating, a citation (growth targets: [`COMPONENTS.md`](COMPONENTS.md)). Needs E6
- [ ] Op-amp, comparator and regulator macromodels and `.subckt` import (LM358, TL431; AMS1117 done in P1), including a licence-recorded import of public unencrypted SPICE `.model` / `.subckt` / `.lib` files, a refusal of encrypted models, and an ngspice-runnable deck export (design: [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md) and [`design/part-creator.md`](design/part-creator.md))
- [x] NE555 timer macromodel (`ti.ne555`, from TI SLFS022K, datasheet checks through the bridge)
- [x] 555 demos in the desktop picker: `ne555-astable`, `ne555-monostable` and `ne555-pwm-dimmer` (1N4148 part), and `esp32c3-555-clock` (a 5 V board with an AMS1117-3.3 and a divider into GPIO4; our own direct-boot firmware counts edges and prints the frequency on UART0), with cold start (`[sim] uic`), house-style layouts and route goldens. Follow-ups: the CMOS TLC555 (a 3.3 V clock with no divider), `circuit fmt` keeping comments, faster debug runs of the long session tests, and the dimmer toward real time (0.36× now)
- [ ] Part creator, first slices C1 and C2 (design: [`design/part-creator.md`](design/part-creator.md)): CLI to create, validate and import/export a `part.toml` pack (including unencrypted SPICE import and an ngspice testbench); then a desktop window to author pins, symbol, cited ratings, variants and licence records. Needs E6 for `[symbol]` (a documented subset may ship first). This is not the circuit schematic editor and not an in-app firmware editor
- [ ] Always-on simulation in the schematic editor (design: [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md)): the circuit keeps solving while the user places parts or edits a value, with an explicit pause; expensive circuits may fall back to a labelled preview. Needs the schematic editor
- [ ] Standard analog waveform file export (SPICE raw or an equivalent documented format) from `ohmforge run` and the scope path, alongside the existing CSV and VCD (same design note). Digital sigrok `.sr` stays on B5
- [ ] Electromechanical parts: DC motor with back-EMF and inertia, relay coil and contacts, servo
- [ ] KiCad schematic/netlist export (`of-kicad`) and XLSX BOM export (design: [`design/kicad-export.md`](design/kicad-export.md); exported wires use the same routed paths). Needs E4, E6
- [ ] Parts availability, first slice (design: [`design/parts-availability.md`](design/parts-availability.md); builds on BOM v1 #59 and E6 #50)
  - [ ] A1 Data model and offline cache: offers, price breaks (`Money` in millionths), lifecycle and alternates in `of-bom`; host-only `of-suppliers` with a SQLite cache, TTLs and `fetched_at`, ECB rates to ZAR, manual import; `ohmforge bom check --offline`; BOM v1's `part.toml` snapshot prices read as dated, lowest-priority offers; an `availability/` dataset kept apart from the CC-BY-SA `part.toml` definitions
  - [ ] A2 LCSC, Mouser and DigiKey providers with the user's own keys in the OS keychain, per-provider rate budgets; `ohmforge parts refresh`
  - [ ] A3 In-app “Check availability” with staleness badges and EOL / out-of-stock warnings, never blocking the simulation
- [ ] Scenario sweeps, AI link and datasheets, first slice (design: [`design/ai-and-scenario-sweeps.md`](design/ai-and-scenario-sweeps.md); no model bundled, no AI vendor SDK linked)
  - [ ] S1 Sweep spec, runner and checks: `of-sweep` TOML sweep files (grid and corner modes over component values and tolerances, supply, temperature, load), measures and pass/fail checks, parallel deterministic runner (`rayon`, per-run seeds), resumable SQLite results store, `ohmforge sweep run` and CSV export. Needs E4, E5
  - [ ] AI1 MCP server, read-only: `ohmforge mcp` (stdio and loopback HTTP with a token) on top of the I1 control API; circuit, readings, BOM and cached availability tools; audit log. Needs I1, E4, A1
  - [ ] AI2 BOM tools and proposals: `bom.check`, `parts.refresh` and alternates through the `of-suppliers` planner and the user's keychain keys and budgets (the model never sees keys); proposed BOM edits confirmed by the user, never overwriting hand entries. Needs AI1, A2
  - [ ] D1 Datasheet sweep: datasheet URL, revision, date, SHA-256 and `fetched_at` for every part in the `availability/` dataset; PDFs cached per user, never committed; revision detection; `ohmforge parts datasheets refresh`. Needs A1 (A2 for supplier URLs)
- [ ] Interconnects W1 (design: [`design/interconnects.md`](design/interconnects.md)): editable wire size in mm² (IEC 60228; AWG per ASTM B258 alongside), material, class, insulation and length per wire, and trace width and copper weight per trace in circuit files; conductor resistance with temperature in the simulation (nets without conductor data stay ideal); voltage-drop readings and warnings; wire lines in the BOM; breadboard jumper defaults. Needs E4, E6
- [ ] Component stress states, first slice (design: [`design/component-stress-states.md`](design/component-stress-states.md))
  - [ ] V1 Stress service: per-component stress from `Health`, `[electrical]` ratings and generic checks; normal, warning, over-limit and damaged states; readings, event log and CLI summary. Needs E2, E5, E6
  - [ ] V2 Schematic visuals: outline, heat tint, shape-coded badges and tooltips with value, limit and source; colour-blind-safe; reduce-motion; optional auto-pause; the normal state unchanged. Needs V1, E4, T1
  - [ ] V3 Paused edits with preview: DC re-solve of an edit while paused, preview of the new stress states, resume or cancel. Needs V1
- [ ] Schematic themes, first slice (design: [`design/schematic-themes.md`](design/schematic-themes.md); today's colours and strokes stay the default; symbols move to IEC by default with T3)
  - [ ] T1 Theme model (TOML, CC-BY-SA-4.0) and the default theme, pixel-identical to today; theme picker with Ohmforge and Paper
  - [ ] T2 Built-in themes and validator: Cyberpunk / Hacker first, Wasteland Terminal (green and amber phosphor; replaces the retro green CRT idea), Blueprint, Dark high-contrast, KiCad-like, colour-blind-safe; WCAG contrast checks; reduce-motion. Needs T1
  - [ ] T3 Symbol styles: IEC 60617 (rectangular resistor; the default, chosen by locale) and ANSI/IEEE 315 (zigzag, today's look) as a separate setting. Needs T1, E6
- [ ] Schematic auto-layout, first slice (design: [`design/schematic-layout.md`](design/schematic-layout.md); feeds the existing router)
  - [ ] L1 Layout metrics and lint: crossings, bends, wire length and best-practice rule checks; `ohmforge layout check`. Needs E4
  - [ ] L2 Full auto-layout: functional grouping, left-to-right signal flow, supply up and ground down, layered placement with port constraints to minimise crossings, power and ground symbols, net labels, decoupling capacitors next to their IC, grid-aligned and deterministic. Needs L1, E6
- [ ] Signal flow and memory, first slice (design: [`design/signal-flow-visualisation.md`](design/signal-flow-visualisation.md))
  - [ ] G1 Schematic levels, current dots and summaries: toggleable overlay with logic-level colouring, current-direction dots, and automatic summarised mode (activity glow, duty-cycle fill, frequency labels) when a signal is faster than about 30 Hz on screen; theme colours; reduce-motion; samples from the live scope ring, not per-step hooks. Needs E4 (demos can start earlier); T1 preferred
  - [ ] G2 Chip view for the C3: block-diagram panel of pads, IO MUX, GPIO matrix, LEDC, SAR ADC, UART0, the interrupt matrix and the CPU, with routes taken from the live register state; hover or click a schematic pin to open its route; activity cues and interrupt flashes. Needs G1
  - [ ] G3 Memory map and hex view: live map of SRAM, RTC, flash windows (MMU translation) and peripheral space; hex/ASCII dump; follow-PC and follow-SP; ELF symbol names; sampled access heat (no per-access hook by default); ROM bytes only if the ROM was fetched; jump from a flash window to the SPI NOR contents. Needs G2
- [ ] Explore compatibility with PCB fabrication services (JLCPCB, PCBWay, OSH Park and similar): export of board designs (Gerber/drill, BOM and pick-and-place/CPL for assembly), likely via the KiCad export (decision recorded in [`design/kicad-export.md`](design/kicad-export.md))
- [ ] Illustrated step-by-step guide to creating your own schematic: screenshots or GIFs for every step, replacing the simple guide ([`src/guide/first-schematic.md`](src/guide/first-schematic.md)) once users can build circuits without Rust; kept current for each release and folded into the P6 user guide. Needs E4 (circuit files), E6 (components and BOM) and the schematic editor's place-and-wire work. The example-gallery and compatibility-check previews ([`src/guide/examples-and-sharing.md`](src/guide/examples-and-sharing.md), [`src/guide/compatibility-checks.md`](src/guide/compatibility-checks.md)) are updated in the same pass.
- [ ] Release 0.2

## P4 Own Xtensa core and web (weeks 40 to 60), exit 0.3: classic ESP32 in the browser

- [ ] Own Rust Xtensa LX6/LX7 core (`of-cpu-xtensa`) replacing the QEMU sidecar where possible
- [ ] Web app (`of-web`, wasm32) running the simulation in the browser, including opening a P3 project pack or share link (design: [`design/examples-and-sharing.md`](design/examples-and-sharing.md))
- [ ] Wi-Fi virtual radio (simulated network between boards and a host bridge) with a shipped fidelity page (design: [`design/wireless.md`](design/wireless.md); official examples run offline; a real-network host bridge is opt-in)
- [ ] MicroPython on the ESP32-C3 (design: [`design/micropython.md`](design/micropython.md)): unmodified official image, UART REPL, one official example; not part of the #60 Arduino/ESP-IDF MVP. Needs I2
- [ ] Sleep modes and a power profiler (current over time, battery life estimate)
- [ ] Energy parts: Li-ion/LiPo with state of charge and internal resistance, coin cell, USB 5 V source
- [ ] Editor plugins (design: [`design/editor-integration.md`](design/editor-integration.md); need P3 items I1 to I4)
  - [ ] I5 VS Code extension MVP (TypeScript, AGPL-3.0-or-later, talks to the control API, never links Rust): start simulator, upload (on build), serial monitor, `ohmforge` debug type via cortex-debug or `cppdbg` against `of-gdb`, task provider, ESP-IDF / PlatformIO / Cargo snippets; Marketplace and Open VSX
  - [ ] I6 Arduino IDE 2.x board package: `package_ohmforge_index.json` with an “Ohmforge simulated ESP32-C3” board on the user's installed `esp32:esp32` core and toolchain (not redistributed), port discovery, upload through `ohmforge flash` or esptool, serial monitor and plotter over the virtual port
- [ ] Parts availability A4 ([`design/parts-availability.md`](design/parts-availability.md)): Nexar, element14, TME, Arrow and JLCPCB providers after terms reviews, “find alternates” with variant swap, XLSX supplier sheets and supplier BOM/cart upload files
- [ ] Scenario sweeps, AI link and datasheets, second slice ([`design/ai-and-scenario-sweeps.md`](design/ai-and-scenario-sweeps.md))
  - [ ] S2 Sweep reports and panel: operating-region, shmoo, threshold, histogram, sensitivity and margin plots; `ohmforge sweep report` to HTML, PNG, SVG and CSV; adaptive boundary refinement; in-app sweep panel. Needs S1
  - [ ] AI3 Sweep tools over MCP: validate, propose, run (with run caps) and read sweep results. Needs AI1, S1, S2
  - [ ] D2 Limits table: reviewed abs-max and recommended limits per part with datasheet page references, used by rating checks and stress states; AI-extracted values stay unreviewed until a maintainer confirms them. Needs D1, E6, V1
  - [ ] D3 Datasheet text extraction (page-chunked, MIT-licensed PDF crates; poppler only as an external tool) and limit proposals over MCP. Needs D1, D2, AI1. The part creator (C3) uses this for field-by-field proposals, never an automatic write ([`design/part-creator.md`](design/part-creator.md))
  - [ ] AI4 Rule-based design review: `ohmforge review` (text, JSON, SARIF) and a Design review panel with ERC and checks for missing pull-ups, unterminated CAN, overvoltage on 3.3 V GPIOs, missing decoupling, LEDs without resistors and exceeded ratings; no AI needed. Needs E6, V1
  - [ ] AI5 AI advisory review: models read the circuit, BOM, results and local datasheet text and submit cited findings that are never applied automatically; local (Ollama) or cloud model chosen by the user, with a notice before anything is sent off the machine. Needs AI4, D3
- [ ] Verilog-A compact models through an OpenVAF sidecar, out of process, after `.subckt` import is solid (design: [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md)). Generated objects need a licence review before any are committed; fail clearly if the sidecar is missing
- [ ] Part creator C3 (same design): import and export of a pinned KiCad symbol (and footprint when the schema is ready), sharing mapping tables with [`design/kicad-export.md`](design/kicad-export.md); datasheet-assisted fill from D3
- [ ] Interconnects W2 and W3 ([`design/interconnects.md`](design/interconnects.md)): wire ampacity and self-heating with insulation limits, a minimum-size suggestion; PCB traces with the IPC-2221 fit and an IPC-2152-informed board model, clearance checks against voltage (IPC-2221, IEC 60664-1), via current; copper weight in the PCB export
- [ ] TWAI / CAN, bus item B6 ([`design/buses.md`](design/buses.md)): C3 TWAI controller (CAN 2.0A/B), SN65HVD230 and TJA1051 transceiver parts with CANH/CANL, 120 Ω termination, bit timing, arbitration, error frames and bus-off, multi-node with a second C3 or a scripted node, CAN decoder
- [ ] Component stress states V4 ([`design/component-stress-states.md`](design/component-stress-states.md)): optional realistic damage (off by default, resettable): parts that exceed their absolute maximum fail open or short like real hardware
- [ ] Schematic themes T4 ([`design/schematic-themes.md`](design/schematic-themes.md)): user themes from a folder with live reload, CRT curvature, flicker and bloom effects, theme-aware SVG, PNG and PDF export
- [ ] Schematic auto-layout L3 and L4 ([`design/schematic-layout.md`](design/schematic-layout.md)): tidy selection and incremental layout with pinned parts, before/after preview with metrics and undo; labelled bus lines for I2C, SPI and CAN
- [ ] Signal flow G4 to G7 ([`design/signal-flow-visualisation.md`](design/signal-flow-visualisation.md)): slow-motion edge travel along wires, tied to playback speed (still a metaphor on ideal nets); bus transaction packets from the protocol decoders; chip, schematic and memory views in the web app; per-chip diagrams for classic ESP32 (then S3 and C6), a fuller register inspector, watch addresses and breakpoint-on-write, and a later source-line link
- [ ] Release 0.3

## P5 Realism (weeks 56 to 72), exit 0.4: validation report published

- [ ] Monte Carlo runs over part tolerances, as sweep item S3 ([`design/ai-and-scenario-sweeps.md`](design/ai-and-scenario-sweeps.md)): distributions, Monte Carlo and combined modes, yield with confidence interval, worst-case search
- [ ] Optional GPU acceleration for batch runs (Monte Carlo, `.step` and parameter sweeps; sweep item S5) through `wgpu`, behind a feature flag with CPU fallback. Single live simulations stay on the CPU: their matrices are too small for a GPU to beat the transfer cost.
- [ ] Thermal model (self-heating, ambient, tempco; MOSFET Rds(on) against temperature)
- [ ] Fault injection (opens, shorts, brown-outs, noisy supplies)
  - [ ] S4 Firmware, stimulus and fault sweeps ([`design/ai-and-scenario-sweeps.md`](design/ai-and-scenario-sweeps.md)): firmware images and inputs, stimuli and faults as sweep variables
  - [ ] B7 Bus faults ([`design/buses.md`](design/buses.md)): stuck or open lines, wrong pull-ups, extra capacitance, noise, baud offset, missing CAN termination, as fault kinds and sweep variables
- [ ] Interconnects W4 ([`design/interconnects.md`](design/interconnects.md)): wire and trace inductance and capacitance per length, coupled to the thermal model and the bus rise times
- [ ] IBIS pin and I/O models as an extension of W4 (design: [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md) and [`design/part-creator.md`](design/part-creator.md) C4): table-driven pin behaviour, not a field solver; same licence rules as SPICE models
- [ ] Optional ideal-switch / piecewise-linear mode for converters and LED drivers (same interop note): faster event-style switching, labelled and selectable; the default path and goldens stay SPICE-grade
- [ ] Part creator C4: bind Verilog-A (OpenVAF sidecar) and IBIS models in the same authoring tool, with the same licence records
- [ ] Signal flow G8 ([`design/signal-flow-visualisation.md`](design/signal-flow-visualisation.md)): on nets with non-ideal conductor data, edge travel can follow the lumped delay from W4; ideal nets keep the metaphor and the label
- [ ] Component stress states V5 ([`design/component-stress-states.md`](design/component-stress-states.md)): temperature derating from the limits table and thermal stress from the thermal model
- [ ] Hardware-in-the-loop lab: compare simulation with real boards
- [ ] Supplier price plugins for the BOM, as parts availability A5 ([`design/parts-availability.md`](design/parts-availability.md)): a scheduled GitHub Action, using project Mouser, DigiKey and LCSC keys Stefan holds as Actions secrets (once each supplier's terms are checked to allow it), that opens PRs updating the shareable `availability/` facts (SKU liveness, lifecycle, alternates; never prices or stock), and South African suppliers (Micro Robotics, Communica, DIYElectronics, Mantech, RS South Africa; all five to be asked for permission or a feed) through agreed feeds or manual CSV import; providers from A2 and A4 are the plugins
- [ ] Publish the validation report (simulation against hardware and ngspice); release 0.4

## P6 1.0 (weeks 70 to 84)

- [ ] Stable component API and WASM SDK for third-party parts (`of-component-api`, `of-plugin-wasm`); the in-tree API comes from phase E2, #46 ([`design/engine-components.md`](design/engine-components.md))
- [ ] ESP32-S3 and ESP32-C6 support, including MicroPython for those chips when official images exist ([`design/micropython.md`](design/micropython.md))
- [ ] Editor rich views (I7, [`design/editor-integration.md`](design/editor-integration.md)): live schematic and instruments in a VS Code webview (the `of-web` build), readings in the debugger, and an optional Arduino IDE 2 extension (a trimmed VS Code extension)
- [ ] Complete documentation (mdBook user guide, part-authoring guide, API docs), including the wireless fidelity page ([`design/wireless.md`](design/wireless.md)), the example, sharing and checks guides, and the part-creator guide ([`design/part-creator.md`](design/part-creator.md))
- [ ] Part creator C5 (same design): a standalone build of the authoring tool that does not start a simulation, so a pack can be created, validated and exported without the ESP32 runtime
- [ ] Optional HDL digital blocks as circuit components, simulated out of process (design: [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md)); not a substitute for the ESP32 core or for sandboxed WASM parts
- [ ] BLE (stretch goal), with its own fidelity page or a written decision to defer it past 1.0 ([`design/wireless.md`](design/wireless.md))
- [ ] Release 1.0

## Future ideas (parked)

These are untracked ideas, not scheduled phases. They need design discussion first and are not on the P0 to P6 plan.

### In-app firmware editor

P2 to P6 keep Ohmforge a simulator: firmware is written and built in the user's editor and toolchain, then loaded or uploaded into the model ([`design/editor-integration.md`](design/editor-integration.md)). An in-app sketch editor, compiler or bundled core is not a silent extension of that work.

- [ ] Whether an in-app sketch editor is ever wanted, and if so for whom, after the VS Code and Arduino IDE 2 integrations exist

### Classroom and translations

Official examples and the illustrated guides are the education path on the roadmap. A teacher dashboard, an LMS integration, or a multilingual UI would be a separate product decision.

- [ ] Classroom or LMS features
- [ ] Translations of the app and the user guide

### 3D bench and enclosure CAD

The P3 breadboard is a 2D, true-pitch view of the same circuit ([`design/breadboard.md`](design/breadboard.md)). A 3D bench, enclosure CAD, or a competition-field physics view is not required for 0.2 to 1.0.

- [ ] 3D real-size bench and enclosure export
- [ ] A physics or competition-field view for robots

### Cloud accounts

An optional account for syncing projects and files across devices. That needs server infrastructure, which costs money, so a proper billing model has to be worked out first. Using the app and creating files locally stays free. Any of this work must stay compatible with AGPL-3.0-or-later; the server code would be AGPL too.

- [ ] Optional account and cross-device sync, with local use remaining free
- [ ] Billing model for the server infrastructure
- [ ] AGPL-3.0-or-later compatibility for any server code

### Radio-frequency analyses

S-parameter and harmonic-balance analyses are useful for matching networks, but they are not required for the P2 to P6 firmware-and-board path. The P4 virtual radio already has a fidelity page ([`design/wireless.md`](design/wireless.md)). Design: [`design/model-and-waveform-interop.md`](design/model-and-waveform-interop.md) §3.8.

- [ ] Whether Ohmforge should grow S-parameter or harmonic-balance analyses, and for which boards, after the virtual radio exists

### Physics interaction layers

Beyond the electrical simulation, model thermal and electromagnetic interactions between components, added over time as needed. Each layer can be turned on or off, with tiered interaction levels (for example beginner, intermediate, advanced, expert, engineer) depending on what the user is testing. The models should follow established physics and electrical theory.

The engine design gives these layers an extension point (optional thermal and coupling ports on components, run-time tiers, telemetry from component readings): [`design/engine-components.md`](design/engine-components.md) section 3j.

Related work already on the P5 checklist, and not replaced by this thread: the thermal model (self-heating, ambient, tempco; MOSFET Rds(on) against temperature), the hardware-in-the-loop lab, and the published validation report.

Open questions, all needing design discussion first:

- [ ] How Ohmforge detects and calculates the interactions
- [ ] How the interactions are shown on screen
- [ ] A telemetry report for the engineer working on the schematic
- [ ] Toggleable layers and the tiered interaction levels
