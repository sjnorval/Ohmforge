# Model and waveform interop

Status: proposed (design only, no code). This note records techniques to add to the plan: live simulation in the editor, cleaner SPICE and compact-model interop, standard analog waveform files, an optional converter-oriented switch mode, IBIS pin models, out-of-process HDL blocks, and a parked radio-frequency analysis thread.
Date: 2026-11-26.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §4 (analog engine), §7 (component model), §9 (UI and instruments); [`part-creator.md`](part-creator.md); [`kicad-export.md`](kicad-export.md); [`interconnects.md`](interconnects.md); [`engine-components.md`](engine-components.md) E5 and E6; [`ROADMAP.md`](../ROADMAP.md).

Ohmforge already has its own analog solver, checked against ngspice as an external golden, a growing `part.toml` library, CSV and VCD logging, and a planned `.subckt` import. The items below fill gaps in how users bring models in, take waveforms out, and keep the editor honest while they are still wiring. They do not change the licence boundary: ngspice, OpenVAF and any GPL or LGPL tool stay out of process; Ohmforge never links them.

## 1. Current state

- The analog solver exports a SPICE deck (`to_spice`) for goldens and can flatten `part.toml` subcircuits. `ohmforge run` writes CSV and VCD. There is no documented, licence-recorded import path for a third-party `.lib` or `.model`, and no standard analog waveform file (for example the common SPICE raw format) alongside VCD.
- Encrypted or term-restricted vendor models must not be committed (`DESIGN_AND_PLAN.md` §7 already says to reference them by URL and checksum and fetch them locally). The rule for *encrypted* files is not yet explicit, and there is no pack-level licence record for a bound model file.
- The desktop demos run live, with play, pause, a pot and a button, but there is no schematic editor yet, so there is also no “keep solving while I place and wire” behaviour.
- Verilog-A compact models, IBIS pin models, an ideal-switch converter mode, HDL blocks used as circuit components, and S-parameter or harmonic-balance analyses are not in the plan today.

## 2. Goals and non-goals

Goals:

- State each technique, the phase it belongs in, and the acceptance bar, so later work does not have to rediscover them.
- Keep licence hygiene first: unencrypted public models with a recorded licence; encrypted models refused; restrictive models fetched by the user and never committed; GPL and LGPL tools only as sidecars, like the planned QEMU bridge.
- Prefer formats that already appear in Ohmforge or in open standards (SPICE netlists and raw files, VCD, Verilog-A, IBIS, Verilog/VHDL) over inventing a parallel ecosystem.

Non-goals:

- Replacing the own analog solver with ngspice, Xyce or any other engine. Those may remain oracles, or sidecars, only.
- Decrypting, reverse-engineering or shipping encrypted vendor model files.
- Making Ohmforge a radio-frequency or IC-layout tool.
- Linking OpenVAF, ngspice, a Verilog simulator or any other GPL/LGPL binary into the workspace.

## 3. The techniques

### 3.1 Always-on simulation in the editor (P3)

When the schematic editor lands, the default should be that the circuit keeps solving as the user places parts, moves a pot or edits a value, with an explicit pause. Today's demos already play live; the editor must not step backwards to a run-only workflow. A paused edit can still offer the DC preview already planned for stress states (V3). Heavy transient work may drop to a cheaper preview (for example a DC operating point, or a cap on live timestep cost) and say so, rather than silently freezing the UI.

Acceptance: in the editor, placing a resistor or turning a pot updates voltages and currents without a separate Run click; Pause stops the solve; an expensive circuit shows a clear “preview only” or paused state instead of dropping frames without explanation.

### 3.2 Vendor SPICE import, licence records, and a runnable deck (P3)

Extend the planned `.subckt` import:

- Accept public, unencrypted `.model`, `.subckt` and `.lib` files, bind them from `[analog]` (as `ti.ne555` and `ams.ams1117-3.3` already do with in-tree macromodels), and record the source URL, retrieval date, checksum, licence and whether the file may be redistributed.
- Refuse encrypted or obfuscated model files with a clear error. Do not attempt to unwrap them.
- Models whose terms forbid redistribution stay out of the tree: store a URL, checksum and licence note, and let a pinned fetch script (the same pattern as the ESP ROM) retrieve them onto the user's machine.
- Export an ngspice-runnable deck for a part or a circuit, so a user can cross-check a result in their own ngspice install. This is an *export* of Ohmforge's view of the netlist, not a claim that every Ohmforge device has a perfect ngspice equivalent; unsupported devices are listed in the export log.

The part creator ([`part-creator.md`](part-creator.md)) is the interactive front end for this path; the CLI (`ohmforge part import-spice`, name *planned*) is the scriptable one.

Acceptance: an unencrypted public subcircuit imports, validates and simulates in Ohmforge; the same testbench exports a deck that ngspice can run; an encrypted file is rejected; a restrictive model is referenced, not committed.

### 3.3 Standard analog waveform files (P3)

`ohmforge run --log` already writes CSV and VCD. Add a commonly used analog waveform format (the SPICE raw plot format is the first candidate) so existing viewers can open an Ohmforge transient, DC or AC run without a CSV conversion step. VCD and the later sigrok `.sr` export (B5) stay for digital. The scope in the app remains first-class; this is interop, not a replacement.

Acceptance: a documented analog waveform file from an official demo opens in at least one external viewer or a small in-tree reader test; units and timescale are preserved well enough to overlay with the CSV log.

### 3.4 Verilog-A compact models through an OpenVAF sidecar (P4)

After `.subckt` import is solid, allow a part to name a Verilog-A source that is compiled out of process by OpenVAF (or a later, licence-compatible successor) to an OSDI or equivalent object, then loaded by a reviewed host path. This is how many modern compact models are published. The generated artefacts and the compiler stay out of the AGPL tree unless a separate licence review says a particular output may be committed. Fail closed if the sidecar is missing.

Acceptance: one public, permissively licensed Verilog-A compact model compiles out of process and matches a cited datasheet check or an ngspice/OSDI golden within a documented tolerance; missing OpenVAF produces a clear error, not a link of a GPL compiler.

### 3.5 Ideal-switch / piecewise-linear converter mode (P5)

Switched-mode supplies and LED drivers are stiff for a continuous Newton–Raphson transient. An optional mode that treats ideal or piecewise-linear switches as events (and keeps the SPICE-grade path as the default) would make converters cheaper to run without dropping the 1 % ngspice bar on the reference set. The two modes must be selectable and labelled; goldens stay on the SPICE-grade path unless a test is explicitly marked for the switch mode.

Acceptance: a documented buck or LED-driver fixture runs substantially faster in the switch mode than in the default transient, and a SPICE-grade golden of the same circuit still passes; the UI or CLI makes the mode obvious.

### 3.6 IBIS pin and I/O models (P5)

The interconnects plan already adds conductor resistance (W1) and later lumped L and C (W4). IBIS (I/O Buffer Information Specification) is the usual way vendors describe pin output/input behaviour for signal integrity. A later slice can bind an IBIS model to a part pin or a connector, still as lumped or table-driven behaviour, not a field solver. This extends [`interconnects.md`](interconnects.md); it does not replace W4.

Acceptance: a documented IBIS model on a fixture pin changes edge rate or clamp behaviour in a testable way; a missing or unreadable IBIS file is an error with a line number, not a silent ideal pin.

### 3.7 HDL digital blocks, out of process (P6)

Community digital parts may be described in synthesizable Verilog or VHDL and simulated by an out-of-process sidecar (the same licence pattern as QEMU). This is for custom digital glue and test benches, not a substitute for the ESP32-C3 core or for sandboxed WASM parts. The component API already planned for P6 is the in-process path; an HDL sidecar is an additional, optional backend.

Acceptance: a small, in-tree test Verilog (or VHDL) block, run out of process, drives a pin in a circuit file with deterministic timing; the sidecar is optional at build time; no GPL or LGPL simulator is linked.

### 3.8 S-parameters and harmonic balance (parked)

S-parameter and harmonic-balance analyses matter for radio-frequency matching, but they are not required for the P2 to P6 ESP32 firmware-and-board path. The P4 virtual radio already has a fidelity page ([`wireless.md`](wireless.md)). This thread stays under Future ideas until there is a concrete need (for example antenna-matching on a development board) and a design discussion.

## 4. Phasing summary

| Item | Phase | Notes |
|---|---|---|
| Always-on simulation in the editor | P3 | With the schematic editor; Pause stays first-class |
| SPICE import, licence records, refuse encrypted, ngspice-runnable deck | P3 | Tightly coupled to the part creator and the existing `.subckt` item |
| Analog waveform file (SPICE raw or equivalent) | P3 | Beside existing CSV/VCD; sigrok `.sr` stays on B5 |
| Verilog-A via OpenVAF sidecar | P4 | After `.subckt` import; licence review of generated code first |
| Ideal-switch converter mode | P5 | Optional; default remains SPICE-grade |
| IBIS pin models | P5 | Extension of interconnects W4, not a field solver |
| HDL blocks out of process | P6 | Optional sidecar; WASM parts remain the in-process community path |
| S-parameters / harmonic balance | Parked | Future ideas, not a 0.2–1.0 requirement |

## 5. Open questions

- The exact analog waveform format and version (SPICE raw is the leading candidate; confirm a reader we can test in CI without a GPL dependency).
- Whether Verilog-A compiled objects are cached per user only, or whether a permissively licensed, reviewed object may be committed next to a `part.toml`.
- How far the ideal-switch mode may drift from the SPICE-grade waveform before a warning is shown.
