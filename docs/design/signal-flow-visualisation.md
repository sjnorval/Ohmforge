# Signal flow and memory: seeing where signals go

Status: proposed (design only, no code). Nothing in this document is implemented yet; section 1 lists what exists today. Phase G0 is this document (P2). G1 to G3 are planned in P3 / 0.2, G4 to G7 in P4 / 0.3 and G8 in P5 / 0.4 (section 7).
Date: 2026-11-24.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §5 (accuracy tiers), §9 (user interface, logic analyser, scope); [`engine-components.md`](engine-components.md) (base engine, instruments); [`interconnects.md`](interconnects.md) (ideal nets today, parasitics in W4); [`buses.md`](buses.md) (pin-level edges, transaction packets, decoders); [`component-stress-states.md`](component-stress-states.md) (additive visuals, reduce-motion); [`schematic-themes.md`](schematic-themes.md) (canvas-only theme colours); [`wire-routing.md`](wire-routing.md) (routed net paths, schematic look and feel); [`editor-integration.md`](editor-integration.md) (I3 `of-gdb`, later source-level debug); [`arduino-idf-mvp-demos.md`](arduino-idf-mvp-demos.md).

Stefan asked to see where signals go inside the ESP32, and where they move on the schematic, while noting that motion could be impractical at high speeds. He also asked to be able to view the memory module. This plan covers three linked views: a live block diagram of the chip that lights the path a signal actually takes, motion and level cues along the schematic wires, and a memory map and hex viewer of what the CPU and the flash chip contain.

## 1. Current state

| Piece | Where it lives today | What it does |
|---|---|---|
| GPIO, GPIO matrix and IO MUX | `of-soc-esp32c3::gpio` (TRM chapter 5) | 22 pins (GPIO0 to GPIO21). `OUT_SEL` 128 drives `GPIO_OUT`; `OUT_SEL` 45 to 50 drive LEDC channels 0 to 5, with inversion and output-enable bits. IO MUX `MCU_SEL` 1 is the matrix; function 0 is a pin's direct function (UART0 TX on GPIO21 after reset). `IN_SEL` routes 128 peripheral inputs (or a constant 0 or 1). `PinOut::thevenin` models drive and the ~45 kΩ pulls. **No modelled peripheral yet reads `peripheral_input`.** There is no API that returns a pin's live route as a path of blocks. |
| Interrupt matrix | `of-soc-esp32c3::intmatrix` | 62 sources to 31 CPU lines, with priority, threshold, edge or level, and clear. Modelled sources include GPIO (16, 17), UART0 (21), LEDC (23), timers, SAR ADC (43) and SYSTIMER. Traps enter vectored `mtvec`. No UI flash when a source fires. |
| LEDC | `of-soc-esp32c3::ledc` | Four timers, six PWM channels, exact `SignalChange` edges on GPIO matrix signals 45 to 50. The hart stops at those edges. |
| SAR ADC | `of-soc-esp32c3::saradc` | One-time sampling. ADC1 channels 0 to 4 are GPIO0 to GPIO4; ADC2 channel 0 is GPIO5. These are **fixed analog pads**, not GPIO-matrix inputs. Pad voltages come from the analog engine; the result is eFuse-calibrated. DMA, filters and the temperature sensor are not modelled. |
| UART0 | `of-soc-esp32c3::uart`, port `uart0` | TX bytes are captured instantly (no pin-level TX waveform). RX is baud-timed into a FIFO, with RX interrupts. Default pins are GPIO21 (TX, IO MUX function 0) and GPIO20 (RX). Pin-level TX and UART1 are bus item B4. |
| SPI flash and cache MMU | `flash`, `spi_mem`, `cache`, `map` | A 4 MiB SPI NOR chip behind SPI0 (cache) and SPI1 (software commands). EXTMEM and a 128-entry MMU, shared by the IROM (`0x4200_0000`) and DROM (`0x3c00_0000`) 8 MiB windows, map 64 KiB virtual pages to 64 KiB flash pages. **No cache lines or miss timing**: a window read goes straight through the MMU. Maintenance operations complete at once. `SocBus::peek` and `mmu_lookup` read flash windows without side effects. |
| Memory map | `of-soc-esp32c3::map`, `SocBus` | Internal ROM (384 KiB at `0x4000_0000`, data-bus alias of ROM1 at `0x3ff0_0000`), SRAM through IRAM (`0x4037_c000`) and DRAM (`0x3fc7_c000`) — 400 KiB, first 16 KiB of IRAM is the ICache region, same bytes — RTC fast (8 KiB at `0x5000_0000`), peripheral space (`0x6000_0000`, 836 KiB). Unmodelled peripheral registers read back the last write and are counted in `SocBus::unmodelled`. There is no access-heat counter and no hex viewer. |
| ROM | `of-soc-esp32c3::rom`, `of-ui::rom` | The real revision 3 mask ROM is fetched from esp-rom-elfs (pinned, checksummed, **never committed**). The GUI already refuses to invent it: demos that need it explain how to fetch it. `RomImage` carries ROM symbols for diagnostics. |
| Debugger and ELF | `of-cpu-riscv::Hart`, `of-firmware::ElfImage` | The hart exposes `pc`, `reg` / `regs` (x2 is the stack pointer), CSR peek/poke, execution breakpoints and `disassemble_at`. `ElfImage` parses `.symtab` (name, value, size). DWARF line info is still a P2 leftover. `of-gdb` is a placeholder (editor item I3). There are no watchpoints or breakpoint-on-write. |
| Engine and nets | `of-engine`, `of-analog`, `Esp32c3Engine` | `of-engine` elaborates components into the analog solver. **Every net is one ideal node**: all pins on a net share one voltage; a wire has zero resistance, inductance and capacitance, and no propagation delay ([`interconnects.md`](interconnects.md)). Live demos still run through `Esp32c3Engine` and an `of-ui` worker thread. |
| Live scope ring | `of-ui::trace::TraceBuf`, `of-ui::live` | A fixed-capacity ring of `(t, value)` samples, capped at 32,768 points, default 50 µs step, down to 1 µs when zoomed. Playback speed runs from `1e-6×` to `10×`. The worker never ticks on the UI thread; the UI paints a snapshot. `of-instruments` is still a stub aside from the serial monitor (measurement service is E5 in P3). |
| Schematic UI | `of-ui` (`wiring`, `theme`, `schematic`, demo schematics) | Routed orthogonal wires with hops; `NetKind` is Signal, Power or Ground. Colours are scattered constants (`TEAL` for live signals and VDD, muted wires, `LED_RED` for a lit LED). Pins and the U1 body have hover tooltips; clicking a pin does not open a route. Demo boards do not draw the module's external flash chip. There is no logic-level colouring, no current-direction dots and no edge travel. |

There is no chip-view panel, no schematic signal animation, and no memory view.

## 2. Goals and non-goals

Goals:

- A **chip view**: a block-diagram panel of the ESP32-C3 (pads, IO MUX, GPIO matrix, peripherals, interrupt matrix, CPU, buses, cache/MMU, on-chip memories and the external flash) that lights the live route of each signal from the **actual simulated register state**, not from a hand-drawn path.
- A **schematic view** that shows where signals move along the already-routed nets: logic-level colouring of digital nets, current-direction dots scaled to magnitude, and, when it is slow enough to watch, an edge pulse travelling along a wire.
- Honest handling of **high speeds**: because wires are ideal, motion is a visual metaphor, not a simulated delay. Above a perceptual threshold (about 30 Hz on screen) the view switches to a summary (activity glow, toggle-rate heat, duty-cycle fill, frequency labels, and later bus transaction packets). Slow-motion follows the existing playback speed, down to `1e-6×`.
- A **memory view**: a live map of the C3 address space, a hex/ASCII viewer with sampled read/write heat, follow-PC and follow-stack-pointer, ELF symbol names where the image has them, and a link from the flash window (and, later, a flash symbol on the schematic) to the bytes on the SPI NOR chip.
- **Toggleable**, theme-aware and kind to the schematic look: the default canvas is unchanged until the user turns a flow overlay on; colours come from the active schematic theme (canvas only); stress-state outlines and badges stay on top; reduce-motion uses static summaries.
- **A performance budget**: no dropped UI frames; sample from the scope ring and a committed-step snapshot, not a hook on every solver step or every bus access.

Non-goals:

- A photograph or an accurate floorplan of the silicon die.
- Copying figures from the Technical Reference Manual, or using third-party artwork.
- A full logic-analyser or scope replacement (those stay instruments; this view points at them).
- Transmission-line simulation or true time-of-flight on ideal wires.
- Bundling or committing ROM or bootloader blobs.
- Stopping the hart on every memory access in order to colour a hex cell.

## 3. Chip view

A dockable panel (beside or below the schematic, not painted over the board) shows a simplified block diagram of the chip in the active canvas theme. Block positions and ports come from **per-chip diagram data** (a TOML or JSON file licensed CC-BY-SA-4.0 with the docs): the painter is generic, so classic ESP32, S3 and C6 can ship their own files later (G7) without a new drawing path.

### 3a. Route derivation

A `SignalRoute` is a list of hops (pad, IO MUX function, matrix in or out select, peripheral block, bus, CPU, interrupt line, cache/MMU, flash). It is rebuilt from the live SoC registers each UI frame, for example:

- **LEDC on GPIO8** (the blink and fade demos): LEDC channel 0 output (`LEDC_LS_SIG_OUT0`, signal 45) → GPIO matrix `OUT_SEL` for pin 8 → IO MUX `MCU_SEL` 1 → GPIO8 pad. Inversion and output-enable bits are part of the hop.
- **ADC on GPIO4** (the dimmer): GPIO4 pad (analog function, **fixed**; not a matrix input) → SAR ADC1 channel 4 → `APB_SARADC` result register → a CPU read on the peripheral bus. The digital GPIO path of the same pad, if also enabled, is a separate route.
- **UART0 TX** (default): UART0 → IO MUX function 0 on GPIO21, unless firmware has remapped it through the matrix. Until pin-level TX exists (B4), the route still lights from the register state; the schematic wire shows a summarised or transaction cue rather than a bit waveform.
- **GPIO input**, for example a button on GPIO9: pad → IO MUX (`FUN_IE`) → `GPIO_IN` and, if configured, `IN_SEL` to a peripheral, plus the GPIO interrupt source into the interrupt matrix and the mapped CPU line.

Hover or click a pin on the schematic, or a pad or block in the chip view, to focus that route and dim the others. A short caption lists the hops, the last level or value, and the toggle rate. Clicking an interrupt source or a flashing CPU line does the same for that path.

### 3b. Activity and the register inspector

Each hop can show a compact activity cue: last logic level or sample, edge count in a recent window, and toggle rate. When an interrupt source in `INT_ST` becomes pending, that source, its mapped CPU line and the CPU block flash briefly (static highlight if reduce-motion is on).

An optional **register inspector** (first slice can be a read-only decode of the registers that built the focused route; a fuller per-block inspector is G7) shows the fields behind the hops: `MCU_SEL`, `OUT_SEL` / `IN_SEL`, LEDC duty and timer, ADC channel and attenuation, UART baud, MMU entries, and so on. Unmodelled blocks stay visible but muted, and the inspector can say that a register is only stored (`SocBus::unmodelled`).

## 4. Schematic view

Overlays are painted on the existing routed paths (`of-ui` `Route` / `Hop` geometry). They are additive: with the overlay off, snapshots match today.

### 4a. Modes

Because a net is one ideal node, a pulse that “travels” along a wire is a metaphor. The view says so in the UI copy, and it never claims to show propagation delay until G8.

| Mode | When it is used | What the user sees |
|---|---|---|
| Live levels | Digital nets slow enough to read, or any net when paused | High / low / high-Z / unknown colour on the stroke, from the theme's net or logic colours |
| Current dots | Analog and supply paths, and digital nets that carry a measurable current | Dots along the route, direction from the sign of the branch current, size or spacing from magnitude |
| Slow-motion edges (G4) | The user slows playback, or a single edge is being inspected | A pulse moves along the wire in lock-step with playback (down to `1e-6×`) |
| Summarised | On-screen event rate above the threshold (default about 30 Hz), or reduce-motion | Activity glow or toggle-rate heat, duty-cycle fill for PWM, a frequency (and duty) label, and later bus “packets” (G5) from the buses design |

The threshold is measured **on the screen**, so it tracks playback speed: a 1 kHz PWM at `1e-3×` is 1 Hz on screen and can show edges; at `1×` it is summarised. The switch is automatic, with a manual override, and is configurable.

Current on an ideal net is not a property of the shared node. Each routed segment takes the current of the branch at that end (the pin or the two-terminal part it meets), following KCL. Magnitude scales the dots; direction follows the sign. Until interconnects split a net into several nodes (W1), there is no voltage drop along the stroke, only this branch current.

### 4b. Themes, stress, motion and performance

- Colours and glow come from the active schematic theme ([`schematic-themes.md`](schematic-themes.md)); themes already apply to the canvas and instruments only, not the app chrome. A monochrome theme (Wasteland Terminal) should use brightness and dash pattern, not a second hue, for levels and heat.
- Stress outlines, tints and badges ([`component-stress-states.md`](component-stress-states.md)) stay on top and are never replaced by flow colour.
- Reduce-motion turns off pulses, travelling edges, interrupt flashes and glow animation; summarised, static cues remain.
- The overlay is a toggle (off by default, section 10). It must not change router geometry or symbol shapes.
- **Performance**: each UI frame reads the latest worker snapshot and, for traces, `TraceBuf` (and the E5 measurement service once it exists). Do not install per-step or per-edge hooks on the solver or the hart just to feed this view. LEDC and GPIO already stamp exact edges for the engine; the overlay samples those, it does not subscribe at a finer rate. A budget: the overlay adds well under a millisecond per frame on the reference machine, and the live demos keep their frame rate.

## 5. Memory view

A second panel (or a tab next to the chip view) shows the C3 as a memory map and a hex/ASCII dump.

### 5a. Map and dump

The map lists the regions in section 1: internal SRAM (IRAM/DRAM alias, with the ICache slice marked), internal ROM, flash through the cache windows (each MMU page shown as a window offset → flash offset, or “invalid”), RTC fast memory, and peripheral register space, broken into the modelled blocks in `map.rs`. Selecting a region or typing an address opens a hex/ASCII view of those bytes, read with `SocBus::peek` (no side effects). Peripheral words can be shown as a register decode when a block is modelled, otherwise as raw values.

**Follow PC** keeps the instruction window and the dump centred on `Hart::pc`, with a disassembly line (`disassemble_at`) and, when the loaded ELF or ROM has a symbol covering that address, its name (`ElfImage::symbols`, `RomImage::symbols`). **Follow SP** does the same for `x2`. The user can pin a region, or jump from a symbol name.

Watch addresses and breakpoint-on-write are later (G7), reusing the I3 GDB stub where possible rather than a second breakpoint mechanism. Execution breakpoints already exist on the hart.

### 5b. Activity heat, sampling and the flash chip

Recent reads and writes light the map and the dump (a heat colour that decays). This must not hook every `Bus` access on the default path: that is the CPU hot path that the realtime work (#61) depends on. By default, a cheap sampled page or line bitmap is updated at the end of a UI quantum, or from a low-rate subsample. Dense per-access heat is only for the page on screen, a watch, or an explicit trace mode.

The external 4 MiB `SpiFlash` is the same object the cache windows read. Choosing an IROM/DROM address, an MMU entry, or (once a symbol exists) the flash chip on the schematic, opens the corresponding flash bytes and shows the translation. Demo boards do not draw that chip today; G3 does not require adding it (section 10). Firmware writes through SPI1, and loader back-doors, appear as write heat on the flash region.

### 5c. ROM contents

The ROM region is shown **only when the user has fetched the pinned ROM**. If it is absent, the map lists the region as “ROM not fetched” with the same fetch instructions the GUI already uses; the dump stays empty and no placeholder bytes are invented. Ohmforge still never commits or bundles ROM or bootloader blobs.

## 6. Tie-ins

- **Interconnects.** Until W1, wires stay ideal and motion is a metaphor. When W4 adds inductance and capacitance, G8 can show a real (lumped) delay on nets that have conductor data, still labelled as a lumped model, not a field solve.
- **Buses.** Pin-level edges from B2 to B4 feed logic-level colouring and summaries. G5 draws short “transaction” packets on a bus net (address, ack, a byte count) from the B5 decoders, instead of thousands of edges.
- **Stress states.** Flow colour never hides a warning, over-limit or damaged state.
- **Scope and logic analyser.** A click on a net or a hop can offer “open in scope” / “open in analyser” once E5 and B5 exist. This view is an overview, not a second instrument.
- **Firmware debugging.** Later (G7, after I3 and DWARF), a hop or a memory row can name the source line and the peripheral driver where possible. That is a link into the editor integration, not a second debugger.

## 7. Phasing

| Item | Phase | Work | Depends on | Acceptance criteria |
|---|---|---|---|---|
| **G0** Design | P2 (now) | This document; roadmap, README, mdBook and changelog pointers. | — | Reviewed and merged by Stefan. No code change. |
| **G1** Schematic levels, current dots and summaries | P3 | Toggleable overlay: logic-level colouring, current-direction dots, automatic summarised mode above the on-screen threshold (glow / heat, duty-cycle fill, frequency labels); theme colours; reduce-motion; samples from `TraceBuf` / the live snapshot (E5 when it lands). | E4 routed circuit files (demos can land earlier on the hand-written boards); T1 theme model preferred but today's constants are enough to start | With the overlay off, blink / dimmer / fade / button snapshots match today. On the blink demo, GPIO8 and the LED net show High/Low in step with the pin. On the dimmer, current dots on the LED path scale with the wiper and do not draw a travelling edge at 1 kHz at 1×; at a playback speed that brings the on-screen rate under the threshold, edges or a duty fill appear. Reduce-motion is static. Frame time stays within the budget. |
| **G2** Chip view for the C3 | P3 | Block-diagram panel from C3 diagram data; live routes for GPIO / IO MUX / matrix, LEDC, SAR ADC, UART0, the interrupt matrix and the CPU; hover or click a schematic pin to focus its route; activity cues and interrupt flashes; a read-only field decode for the focused route. | G1 for the shared snapshot; the SoC register state as it is today | Fade lights LEDC ch0 → matrix signal 45 → GPIO8. Dimmer lights GPIO4 pad → ADC1 channel 4 → APB → CPU, without inventing a matrix hop. A button edge lights GPIO9 → GPIO interrupt → the mapped CPU line and flashes that source. Changing `OUT_SEL` in a test fixture moves the drawn route. Unmodelled blocks stay muted. |
| **G3** Memory map and hex view | P3 | Live map of SRAM, RTC, flash windows (with MMU translation), peripheral space; hex/ASCII dump via `peek`; follow-PC and follow-SP; ELF symbol names; sampled access heat; ROM region only if the ROM was fetched; jump from a flash window to `SpiFlash` bytes. | G2 panel chrome (can share a tab); `ElfImage` symbols; existing `peek` / `mmu_lookup` | Follow-PC on the blinky tracks the instruction stream and shows a symbol when the ELF has one. An IROM address shows the same bytes as the flash page the MMU maps. With no ROM fetched, the ROM region explains how to fetch it and shows no bytes. Heat appears on a recently written SRAM or flash page without hooking every bus access (a test that runs a tight loop does not regress the hart hot path). |
| **G4** Slow-motion edge travel | P4 | Pulses travel along the routed wire in lock-step with playback speed (down to `1e-6×`); still labelled as a metaphor on ideal nets. | G1 | At `1e-3×` on a 1 kHz PWM, a pulse can be watched along GPIO8's route; at 1× the same net stays in summarised mode. |
| **G5** Bus transaction packets | P4 | Short packet markers on I2C, SPI, UART and, when B6 lands, CAN nets, from the protocol decoders. | G1; B5 (and B6 for CAN) | An I2C write shows a packet with address and length instead of every SCL edge when summarised; opening the analyser still shows the edges. |
| **G6** Web app | P4 | Chip view, schematic overlay and memory view in `of-web`, same data model as the desktop. | G1 to G3; the P4 web app | The dimmer demo in the browser shows the GPIO4 ADC route and the memory map, with the overlay toggle off leaving the schematic unchanged. |
| **G7** Other chips, inspector, watches | P4 | Per-chip diagram data for classic ESP32, then S3 and C6 as those SoCs land; fuller per-block register inspector; watch addresses and breakpoint-on-write (with I3); optional link from a hop or a memory row to a source line once DWARF exists. | G2, G3; I3 for watches; DWARF for source lines | A classic ESP32 diagram lights its own matrix and peripherals from its registers. A write watch on an SRAM word pauses the run on the write, not on a read. ROM rules in G3 still hold. |
| **G8** Real propagation | P5 | On nets with non-ideal conductor data, edge travel can follow the lumped delay from W4; nets without conductor data keep the metaphor and the label. | G4; W4 | A long resistive-plus-LC lead shows a delay consistent with the W4 transient; an ideal net still captions the motion as a metaphor. |

## 8. Invariants

1. **The default schematic looks exactly as today.** Flow, heat and packets are an overlay the user turns on.
2. **Routes come from simulated register state**, not from a hard-coded demo path. Diagram layout may be curated; connections may not.
3. **Ideal wires have no propagation delay.** Any travel along them is a metaphor until G8, and is labelled as such.
4. **Do not hook every bus access or solver step** on the default path. Sample.
5. **No ROM or bootloader blobs are committed.** The ROM dump is empty until the user has fetched the pinned image.
6. **Reduce-motion disables animation**; static level, heat and summary cues remain.
7. **Stress visuals stay visible** and are not re-coloured away by the overlay.

## 9. Licensing

- This design and the per-chip diagram data are CC-BY-SA-4.0, like the other docs and parts. The code that implements it is AGPL-3.0-or-later.
- Draw original block diagrams. Do not copy TRM figures, vendor die photos or third-party game or film artwork.
- Dependencies stay AGPL-compatible. No ngspice, no new GPL-linked decoder libraries.
- ROM and second-stage bootloader files stay fetched, pinned and checksummed, never committed, as today.

## 10. Decided and open questions

Decided (24 November 2026, Stefan):

- **Schematic overlay** (former open question 1). Off by default. Today's look stays the default, as with themes and stress visuals. Chip view and memory view are panels the user opens. A single “Signal flow” toggle (with the automatic summary) is enough for the first slice.
- **Auto-summary threshold** (former open question 2). 30 Hz on screen by default, configurable. It is measured after playback speed is applied. Reduce-motion always uses the static summary, regardless of the threshold.
- **Hex view writes** (former open question 3). Read-only while running. While paused, optional debugger pokes, off until the user enables them, and clearly labelled. Same idea as a GDB memory write (I3).
- **Flash symbol on demos** (former open question 4). Not in G3. The memory panel and the chip-view flash block are enough. A package flash symbol can wait for circuit files (E4) and a part in the library, or a later “show module flash” option.
- **Access heat** (former open question 5). A sampled page or line bitmap by default, plus dense heat only for the visible page, a watch, or an explicit trace mode. A tight loop in SRAM must not regress the hart hot path.
- **Panel placement** (former open question 6). Dockable side or bottom panels, sharing a tab strip, not a modal over the schematic. Clicking a pin focuses the route and can open the panel if it is closed.

Open questions: none at present.
