# Roadmap and task list

Derived from `docs/DESIGN_AND_PLAN.md` section 14. Tick a box only when the work is verified, all checks in `AGENTS.md` pass and it is committed; `[~]` means partly done (what is missing is noted on the item). Work on a feature branch counts once it is pushed with green CI. Status as of 23 November 2026 (`main` includes PRs #7 to #55; the first were #7 to #12: P1, the blinky milestone, interrupts, timers and watchdogs, the ESP image parser and the desktop blink demo; P2 is in progress). The ROM boot work (branch `p2-rom-boot`) is on `main` (#20), and the esp-hal blinky (branch `p2-esp-hal-blinky`) too (#21). GPIO inputs (branch `p2-gpio-inputs`) are on `main` (#22). ADC and LEDC PWM (branch `p2-adc-pwm`) are on `main` (#24). The desktop demo picker (#26) and live playback, bench supply and virtual meters (#27) are on `main`, as are faster playback (#28), honest bench I/O (#29), baud-timed UART0 RX (#30), scope zoom (#33), the schematic router with hops (#35 to #41), the EILIi load screen (#40, #42), the engine design (#43), CPU/solver speedups (#44), the docs alignment (#51), green and blue 0603 LEDs (#52), the remaining P0 leftovers (#53) and the PSU return and current-averaging fix (#45, closed by #54), and RFC 0001 for the E2 component API (#55). The parts the library should grow to, and what each one needs from the simulator, are in the [common components checklist](COMPONENTS.md). The move to a base engine with plug-in components (the ESP32-C3, the bench PSU and every part as components on one engine API) is planned in [`design/engine-components.md`](design/engine-components.md) (phases E1 to E3 in P2 and E4 to E6 in P3, below; PSU return #45 done, remaining issues #46 to #50).

**Current phase: P2 ESP32-C3 MVP, in progress** (ESP32-C3 SoC on the RISC-V core, co-simulated with the analog solver through the `of-core` kernel). P1 Analog core and phase one (the RISC-V CPU core, pulled forward from P2) are done; the first P2 slice (blinky end to end), the interrupt matrix, SYSTIMER alarms, timer groups, watchdogs, the ESP image parser, the desktop demo picker, live playback, the bench supply and virtual meters are on `main`. P0 leftovers continue alongside.

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
- [x] `of-soc-esp32c3`: memory map bus (IROM/DROM flash windows, IRAM/DRAM, ROM, RTC fast, peripherals) on top of `Bus` (`SocBus`; cache MMU)
- [~] ESP32-C3 custom CSRs through `Bus::csr_read`/`csr_write`: performance counters `mpcer`/`mpcmr`/`mpccr` and `upcer`/`upcmr`/`upccr` (cycle event only) and the user trap CSRs (stored only); one counter behind both names with `mpcmr` reset to count and saturate, PMP and debug trigger CSRs (stored, not enforced) (#21); other events and the GPIO CSRs to do
- [x] Interrupt matrix model feeding `set_interrupt_lines` (priority, threshold, edge/level; `of_soc_esp32c3::IntMatrix`)
- [ ] Calibrate `TimingModel` against a real ESP32-C3 (cycle counts of benchmark loops, flash cache misses)
- [~] Decode cache (invalidated by `fence.i`, RISC-V I-stream coherence): a 4096-line direct-mapped cache in the hart that skips the bus fetch on hits; monomorphised `Bus` run loop and quiescent-slice GPIO batching (#28, #44); MIPS still to benchmark, and 1× realtime on busy demos still open
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
  - [~] E2 Engine core (`of-engine`) and the `Component` trait (#46): elaboration into `of-analog` devices, registry, invariants (one ground, every source has a return path, KCL, no floating nets), terminal currents; port the resistor, pot, LED, push button and bench PSU (`+` and `−` terminals). RFC Accepted ([`rfcs/0001-component-api.md`](../rfcs/0001-component-api.md)); `of-engine` + `LibraryComponent` / `BenchPsu` landed with parity and KCL tests — demos still assemble in `of-ui` until E3/E4
  - [ ] E3 ESP32-C3 as a component (#47): GPIO half-bridges on its own 3V3/GND pins, input watches, ADC sample points, LEDC, UART port, supply load, firmware and boot parameters, CPU as an active engine; engine glue moves out of `of-ui`. Needs E2
  - E4 to E6 (circuit files, instruments, parts library and BOM): moved to P3 / 0.2, see below
- [ ] egui schematic editor (place, wire, probe), oscilloscope, logic analyser with UART decode, serial monitor
  - [x] Read-only desktop blink demo (`of-ui` `BlinkDemo`, `cargo run -p of-app --features gui`): bundled blinky co-simulated with R1 330 Ω and a red LED, the schematic LED lit from the simulated current, an LED current trace and UART0 output. Not the editor or the real oscilloscope (GPU waveform path, decision d2)
  - [x] Live desktop playback (worker thread, realtime speed control, live pot/button, scrolling scope, clickable SW1, tooltips, configurable bench PSU on the VDD rail, virtual voltmeter/ammeter) on `main` (#27). Window-tied scope retention, 1-2-5 time/div zoom (10 µs/div to 1 s/div) and playback down to 1e-6× are on `main` (#33). The analog solver holds a settled DC solution instead of re-solving every slice, and the hart caches decoded instructions (#28). Further CPU/solver speedups (decode-cache hits skip fetch, monomorphised bus, quiescent GPIO batching; #44) raise release realtime on dimmer to about 0.23× and on button-poll to about 0.17×; 1× realtime on the busy demos is still open. GPIO thresholds track live Vset, and clickable SW1 bounces with the firmware-test train (#29).
  - [~] Serial monitor: lean console in `of-instruments` / `of-ui` (TX log, Send delivers to UART0 RX at the configured baud when attached; hex view and find, #30). Soft-front-panel hosting still to do (d7)
  - [ ] Schematic wire routing (design: [`docs/design/wire-routing.md`](design/wire-routing.md); keeps the existing schematic look). The P3 breadboard view and KiCad export consume the same paths.
    - [x] Data model and blink fixtures (no visual change); golden geometry tests
    - [x] Orthogonal grid router (A*/Lee, stubs, T-junctions, body clearance); blink uses generated paths
    - [x] Port the pot-dimmer and push-button demos
    - [x] Crossing policy (avoid, then hop) and rip-up and reroute
    - [ ] Optional 45° PCB-style mode (off by default)
    - [ ] User-editable waypoints and click-to-wire (editor slice; no editor chrome in the earlier PRs; needs the E4 circuit files and E6 symbols, so it follows them in P3)
    - [ ] Interactive trace re-edit mode in the schematic editor: manually re-route or adjust auto-generated traces (drag segments; move, add or remove waypoints/bends; re-run autoroute on a net); edits keep the same invariants (no body crossings, no foreign-net overlap, T-junctions)
- [ ] BOM v1: CSV with MPN, supplier links, ZAR cost; generated build checklist (for 0.1 from the demo boards' placements; moves onto components with E6, #50, in P3)
- [ ] MVP demo: blink, button interrupt, potentiometer to ADC, LED dimming, all from unmodified Arduino/ESP-IDF/esp-hal ELFs (all four run from esp-hal: blink, the button polled and on an interrupt (#22), and the pot dimmer and LED fade (#24); the desktop app can play the blink, dimmer, fade and push-button demos; Arduino and ESP-IDF builds still to do)

### First milestone: blinky end to end

Each step lands with its own tests; firmware is ours and tiny, built with zig and committed (CI checks it rebuilds byte-identical). Steps 1 to 7 are on `main`.

1. [x] Memory map and bus (`SocBus`): region routing, aliases, read-only ROM/flash, fetch rules (unit tests)
2. [x] ELF reader (`of-firmware`) and direct boot (`Esp32c3::load_elf`): hand-built ELF, truncation and malformed-file tests, real zig-built ELF
3. [x] GPIO + minimal IO MUX/GPIO matrix output path (unit tests per register and alias)
4. [x] SYSTIMER unit 0 read path, UART0 TX capture (unit tests; banner check in the blinky test)
5. [x] Blinky firmware (`firmware/esp32c3-blinky`, RV32IMC asm, SYSTIMER-timed): edges every 250 µs without drift (`tests/blinky.rs`)
6. [x] `Esp32c3Engine` + `PinDriver` + parts-library LED and resistor under the kernel: LED current switches with a 500 µs period, about 4 mA on (`tests/blinky_led.rs`)
7. [x] Same blinky from esp-hal (Rust, `riscv32imc-unknown-none-elf`, #21): `firmware/esp32c3-esp-hal-blinky` (esp-hal 1.2.2, unmodified `esp_hal::init` with `CpuClock::max()`, 500 µs half period) boots through the real ROM and the ESP-IDF bootloader, prints its banner at 160 MHz and drives the LED circuit (`tests/esp_hal_blinky.rs`); CI rebuilds it byte for byte
8. [ ] Arduino-ESP32 / ESP-IDF blink through the real ROM and a flash image (deferred: needs an ESP-IDF or Arduino toolchain in CI; the bootloader path it uses already works)

### MVP definition (exit criteria for 0.1)

- [ ] Unmodified ESP32-C3 ELF (Arduino-ESP32, ESP-IDF or esp-hal) boots from the real ROM
- [ ] Blink, button with interrupt, potentiometer into ADC, LED dimming via LEDC behave as on hardware
- [x] RC filter, LED with series resistor, transistor switch, AMS1117 regulator within 1 % of ngspice
- [ ] Oscilloscope, logic analyser with UART decode, multimeter probe, serial monitor
  - [x] Virtual voltmeter/ammeter on the live schematic (click two probe points; non-disturbing ammeter on a unique branch) on `main` (#27)
  - [~] Serial monitor: lean console (TX log, Send delivers to UART0 RX at the configured baud when attached; hex view and find, #30). d5 shell hosting still to do (d7)
- [ ] BOM CSV export with MPNs, supplier links and ZAR cost, plus build checklist
- [x] EILIi startup animation as the desktop app load screen (packed 24 fps JPEG frames, no video decoder; click/Esc/any key skips; persistent "Show startup animation" setting; `--no-splash` / `OF_NO_SPLASH` still override).
- [ ] Desktop builds on Windows, macOS, Linux; deterministic headless runs in CI

## P3 Breadth (weeks 28 to 42), exit 0.2: common hobby projects run

- [ ] Base engine phases E4 to E6 (moved from P2 on 23 November 2026; design: [`design/engine-components.md`](design/engine-components.md); E1 to E3 are in P2). The schematic editor's waypoint and re-edit work and the component-based BOM build on these.
  - [ ] E4 (#48) Circuit files (serde, round-trip): blink, dimmer, fade and button drive both the simulation and the shared router, with output parity (firmware traces and wiring goldens match or have reviewed updates). Needs E3
  - [ ] E5 (#49) Instruments as components: measurement service in `of-instruments` (trace store, period-aligned averages, duty/frequency), voltmeter, ammeter, scope, logger and UART console on any circuit. Needs E2 (E4 for the desktop panels)
  - [ ] E6 (#50) Parts library and BOM from components: `[symbol]`, pin kinds and `bom` in `part.toml`; the BOM comes from the circuit's components. Needs E4
- [ ] Classic ESP32 (Xtensa LX6) through a QEMU sidecar process (`of-cpu-qemu-ipc`), kept out of process for licence and stability reasons
- [ ] Peripherals: I2C, SPI, RMT, DAC (C3 and classic ESP32)
- [ ] Flash like real hardware: expose the simulated chip's UART0 (and later the C3's USB Serial/JTAG) as a virtual serial port (TCP/RFC 2217 first, then a host COM/PTY bridge), with the board's DTR/RTS auto-reset circuit driving EN and GPIO9 into the ROM's download mode, so unmodified `esptool`, `espflash`, the Arduino IDE and `idf.py flash` write the simulated SPI flash through the real ROM loader; flash contents persist with the project (editor-integration item I2, [`design/editor-integration.md`](design/editor-integration.md))
- [ ] Editor integration foundations (design: [`design/editor-integration.md`](design/editor-integration.md); the plugins follow in P4)
  - [ ] I1 Control API and headless mode: `ohmforge serve` and the same server in `of-app`, JSON-RPC 2.0 over a named pipe / Unix socket or loopback TCP with a session token; open project, load and flash firmware, reset, run/pause, UART and readings; `ohmforge flash` client. Needs E3 (E4 for `project.open`)
  - [ ] I2 Virtual serial port and auto-reset: the “Flash like real hardware” item above
  - [ ] I3 `of-gdb` GDB remote stub (from the phase-one list): breakpoints, registers and memory through the bus, virtual time stopped while halted
  - [ ] I4 Build-folder watch and hot reload (`DESIGN_AND_PLAN.md` §8 step 5): new firmware with the circuit state kept
- [ ] Breadboard view alongside the schematic editor (uses the same routed net paths as the schematic; [`docs/design/wire-routing.md`](design/wire-routing.md))
- [ ] Parts library to about 50 parts (sensors, displays, drivers, regulators), each with a `part.toml`, behaviour and tests (checklist with priorities and simulator dependencies: [`COMPONENTS.md`](COMPONENTS.md))
- [ ] Op-amp, comparator and regulator macromodels and `.subckt` import (LM358, TL431; AMS1117 done in P1)
- [ ] Electromechanical parts: DC motor with back-EMF and inertia, relay coil and contacts, servo
- [ ] KiCad schematic/netlist export (`of-kicad`) and XLSX BOM export (exported wires use the same routed paths)
- [ ] Explore compatibility with PCB fabrication services (JLCPCB, PCBWay, OSH Park and similar): export of board designs (Gerber/drill, BOM and pick-and-place/CPL for assembly), likely via the KiCad export
- [ ] Release 0.2

## P4 Own Xtensa core and web (weeks 40 to 60), exit 0.3: classic ESP32 in the browser

- [ ] Own Rust Xtensa LX6/LX7 core (`of-cpu-xtensa`) replacing the QEMU sidecar where possible
- [ ] Web app (`of-web`, wasm32) running the simulation in the browser
- [ ] Wi-Fi virtual radio (simulated network between boards and a host bridge)
- [ ] Sleep modes and a power profiler (current over time, battery life estimate)
- [ ] Energy parts: Li-ion/LiPo with state of charge and internal resistance, coin cell, USB 5 V source
- [ ] Editor plugins (design: [`design/editor-integration.md`](design/editor-integration.md); need P3 items I1 to I4)
  - [ ] I5 VS Code extension MVP (TypeScript, AGPL-3.0-or-later, talks to the control API, never links Rust): start simulator, upload (on build), serial monitor, `ohmforge` debug type via cortex-debug or `cppdbg` against `of-gdb`, task provider, ESP-IDF / PlatformIO / Cargo snippets; Marketplace and Open VSX
  - [ ] I6 Arduino IDE 2.x board package: `package_ohmforge_index.json` with an “Ohmforge simulated ESP32-C3” board on the user's installed `esp32:esp32` core and toolchain (not redistributed), port discovery, upload through `ohmforge flash` or esptool, serial monitor and plotter over the virtual port
- [ ] Release 0.3

## P5 Realism (weeks 56 to 72), exit 0.4: validation report published

- [ ] Monte Carlo runs over part tolerances
- [ ] Optional GPU acceleration for batch runs (Monte Carlo, `.step` and parameter sweeps) through `wgpu`, behind a feature flag with CPU fallback. Single live simulations stay on the CPU: their matrices are too small for a GPU to beat the transfer cost.
- [ ] Thermal model (self-heating, ambient, tempco; MOSFET Rds(on) against temperature)
- [ ] Fault injection (opens, shorts, brown-outs, noisy supplies)
- [ ] Hardware-in-the-loop lab: compare simulation with real boards
- [ ] Supplier price plugins for the BOM
- [ ] Publish the validation report (simulation against hardware and ngspice); release 0.4

## P6 1.0 (weeks 70 to 84)

- [ ] Stable component API and WASM SDK for third-party parts (`of-component-api`, `of-plugin-wasm`); the in-tree API comes from phase E2, #46 ([`design/engine-components.md`](design/engine-components.md))
- [ ] ESP32-S3 and ESP32-C6 support
- [ ] Editor rich views (I7, [`design/editor-integration.md`](design/editor-integration.md)): live schematic and instruments in a VS Code webview (the `of-web` build), readings in the debugger, and an optional Arduino IDE 2 extension (a trimmed VS Code extension)
- [ ] Complete documentation (mdBook user guide, part-authoring guide, API docs)
- [ ] BLE (stretch goal)
- [ ] Release 1.0

## Future ideas (parked)

These are untracked ideas, not scheduled phases. They need design discussion first and are not on the P0 to P6 plan.

### Cloud accounts

An optional account for syncing projects and files across devices. That needs server infrastructure, which costs money, so a proper billing model has to be worked out first. Using the app and creating files locally stays free. Any of this work must stay compatible with AGPL-3.0-or-later; the server code would be AGPL too.

- [ ] Optional account and cross-device sync, with local use remaining free
- [ ] Billing model for the server infrastructure
- [ ] AGPL-3.0-or-later compatibility for any server code

### Physics interaction layers

Beyond the electrical simulation, model thermal and electromagnetic interactions between components, added over time as needed. Each layer can be turned on or off, with tiered interaction levels (for example beginner, intermediate, advanced, expert, engineer) depending on what the user is testing. The models should follow established physics and electrical theory.

The engine design gives these layers an extension point (optional thermal and coupling ports on components, run-time tiers, telemetry from component readings): [`design/engine-components.md`](design/engine-components.md) section 3j.

Related work already on the P5 checklist, and not replaced by this thread: the thermal model (self-heating, ambient, tempco; MOSFET Rds(on) against temperature), the hardware-in-the-loop lab, and the published validation report.

Open questions, all needing design discussion first:

- [ ] How Ohmforge detects and calculates the interactions
- [ ] How the interactions are shown on screen
- [ ] A telemetry report for the engineer working on the schematic
- [ ] Toggleable layers and the tiered interaction levels
