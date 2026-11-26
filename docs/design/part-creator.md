# Part creator

Status: proposed (design only, no code). This is the chosen deep dive from the 2026-11-26 plan audit: a tool for authoring complete, cited, simulation-ready parts into `part.toml` packs, designed so it can later ship as a standalone tool as well as a command and window inside Ohmforge.
Date: 2026-11-26.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §7 and §13.4; [`COMPONENTS.md`](../COMPONENTS.md); [`model-and-waveform-interop.md`](model-and-waveform-interop.md); [`kicad-export.md`](kicad-export.md); [`compatibility-checks.md`](compatibility-checks.md); [`engine-components.md`](engine-components.md) E6; [`editor-integration.md`](editor-integration.md) (this is not a firmware editor); [`CONTRIBUTING.md`](../../CONTRIBUTING.md#adding-a-part); [`parts/README.md`](../../parts/README.md).

The quality and size of the parts library is the main limit on whether common hobby projects actually run in Ohmforge. Today a good part is a hand-written folder: pins, cited ratings, an analog model or subcircuit, variants with MPNs, and a licence block. That work is slow, easy to get slightly wrong, and hard to offer as a first contribution. This note specifies a **part creator**: a tool that authors a complete part pack and, over time, can stand on its own as a general-purpose part authoring tool for people who also use KiCad or a SPICE deck.

## 1. Why this, and not another plan item

The 2026-11-26 audit added several simulator techniques to the plan (live editing, SPICE and Verilog-A interop, waveform files, an ideal-switch mode, IBIS, out-of-process HDL, and parked RF analyses). Those are all worth doing. The question is which one most improves Ohmforge if we specify it properly now.

- **Always-on simulation in the editor** is an important feel for the schematic editor, but it is blocked on that editor and does not grow the library. It is already captured as a P3 behaviour in [`model-and-waveform-interop.md`](model-and-waveform-interop.md).
- **SPICE import, waveform export, Verilog-A, IBIS, a converter switch mode and HDL sidecars** make models richer. They pay off only when someone can bind a model to a real, orderable part with pins, citations and a symbol. The first of those (unencrypted SPICE import, licence records, an ngspice-runnable testbench) belong *inside* the part creator.
- **Release 0.2 aims at about 50 cited, tested parts** ([`COMPONENTS.md`](../COMPONENTS.md)). The engine, instruments and circuit-file work on `main` are already ahead of the library: E4 is done, E5 is all but follow-ups, and E6 (symbols and a component-based BOM) is still open. The path to fifty parts is authoring, not another solver mode.
- **`DESIGN_AND_PLAN.md` §13.4** already sketches `ohmforge new-part` creating `part.toml`, `symbol.svg` and `breadboard.svg`. This note makes that real: a guided tool, validation, model checks, and later a standalone package.

The part creator is therefore the largest win: it unblocks the library, it absorbs the most useful model-import work, and a careful, licence-clean pack format with KiCad and SPICE interop is something the wider electronics community can use even if they never run an ESP32 simulation. It is not a replacement for the circuit schematic editor, and it is not the parked in-app firmware editor.

## 2. Problem

Authoring a part today means editing TOML by hand, often copying an existing folder, wiring `[analog]` to a `.lib` file, filling `[electrical]` with datasheet figures and a `note`, listing `[[pin]]` and `[[variant]]`, and running `ohmforge validate-parts`. There is no symbol or breadboard artwork schema yet (that is E6), no guided licence record for a third-party model, no push-button check that the macromodel still matches the cited figures, and no import from a KiCad symbol or a SPICE library. Mistakes (a pin order that does not match the subcircuit, a rating without a source, a model whose terms forbid redistribution) are cheap to make and expensive to catch in review.

## 3. Users

- **Ohmforge maintainers and contributors** adding official parts to `parts/` (CC-BY-SA-4.0 data and artwork).
- **Project authors** who need a part that is not in the official library yet, for a circuit they want to simulate, and who may later offer it upstream.
- **People outside Ohmforge** (a later audience) who want a cited, simulation-ready part pack: a symbol, a footprint or breadboard graphic, a pin map, a SPICE binding, and BOM fields they can export into KiCad or keep as a folder. This is the “industry-general” use, and it must not depend on an ESP32 being in the circuit.

## 4. Current state

- The schema lives in `crates/of-parts`. A part folder is `parts/<category>/<manufacturer>-<part>/part.toml` or `parts/<category>/<part>/` for generics. Fields today: `[part]` (id, name, category, datasheet, model, verification), `[[pin]]` (number, name, kind), `[electrical]` ratings with units and notes, `[analog]` (kind, pins, file, subckt, parameters), `[[variant]]` (label, mpn, manufacturer, footprint, offers), `[license]` (`data` and `artwork`, both CC-BY-SA-4.0 in the official library).
- Official parts already in the tree include passives, LEDs, a small-signal NPN, the AMS1117 and NE555 macromodels, a BME280 supply load, a push button, a potentiometer, `espressif.esp32-c3`, `bench.psu`, and instrument profiles under `parts/instruments/`.
- `ohmforge validate-parts` checks schema and some invariants (for example a `subckt` kind needs `file` and `subckt`). Datasheet checks for some macromodels run in Rust tests through the parts bridge, and ngspice is used as an external golden for solver devices. There is no part-level “export this testbench and run ngspice” command.
- `[symbol]` and breadboard artwork are planned in E6; they are not in the schema yet. KiCad export of a *circuit* is planned in [`kicad-export.md`](kicad-export.md); there is no path yet to import or export a *part* symbol.
- `CONTRIBUTING.md` describes the hand-written flow and already says not to commit vendor SPICE models with restrictive terms.

## 5. Goals and non-goals

Goals:

- A repeatable way to create a complete part pack: identity, pins, symbol, breadboard or footprint graphic, electrical ratings with citations, analog or behavioural binding, variants with MPNs, and licence records for data, artwork and any bound model.
- Import from files people already have (unencrypted SPICE, KiCad symbols and footprints, and later a datasheet) and export to `part.toml` plus, where it is lawful, KiCad symbol/footprint libraries.
- Validation that a human can run before a pull request: schema, pin-map consistency with the model, required citations on ratings that checks will use, licence completeness, and a model check against cited figures and, where it applies, against ngspice out of process.
- A packaging shape that can later run as `ohmforge part …`, as a window in the desktop app, and as a standalone binary that does not boot the ESP32 simulator.
- Licence hygiene that is stricter than hope: official contributions stay CC-BY-SA-4.0; user-owned packs may use other licences but must declare them; encrypted models are refused; restrictive models are referenced, not copied.

Non-goals:

- A circuit schematic editor, a breadboard of a whole project, or a PCB layout tool. Those stay in the editor, breadboard and KiCad-export work.
- An in-app firmware or sketch editor ([`editor-integration.md`](editor-integration.md), and the parked future idea).
- Auto-generating a trustworthy model from a datasheet with no review. Datasheet-assisted fill (P4) proposes values and citations; a human accepts them.
- Supporting every historical EDA symbol format, or vendoring KiCad or any GPL library.
- Bundling vendor PDFs or encrypted model files in the Ohmforge tree.

## 6. User-facing behaviour

### 6.1 Start a part

`ohmforge part new <category>/<id>` (the name in §13.4 was `ohmforge new-part`; pick one name and keep it) creates a folder with a skeleton `part.toml`, empty or template `symbol.svg` and `breadboard.svg`, and a test stub. In the desktop app (and later the standalone tool), File → New part opens the same wizard: category, part id (`vendor.part` or `generic.…`), display name, datasheet URL, and whether this is an official contribution or a personal pack.

### 6.2 Edit the pack

The tool shows one part at a time, in panes that map to the file:

- **Identity:** id, name, category, datasheet URL, verification level (`community`, `datasheet`, `bench-measured`), model kind (`rust:…`, `wasm:…`, `spice:…`, later `veriloga:…`).
- **Pins:** a table of number, name and kind (`power`, `ground`, `input`, `output`, `io`, …), plus the order used by the analog model. Reordering pins must update the symbol, the `[analog].pins` list and any subcircuit mapping together, or show a consistency error until the user confirms.
- **Symbol:** a simple symbol editor (pins on a body, labels, a small set of primitives: rectangle, polyline, IEEE/IEC-ish amplifier, ground, and text). The source of truth is the pack file (SVG or a TOML `[symbol]` once E6 lands), not a private binary. Grid, snap, and a preview at schematic scale.
- **Footprint / breadboard graphic:** a 2.54 mm-aware drawing or a footprint id string (`Package_SO:SOIC-8_…` as in `ti.ne555` today). The breadboard view will consume the graphic; KiCad export will consume the footprint id and, later, an exported footprint.
- **Electrical:** each rating is a name, min/typ/max, unit, and a citation (`note` or a structured source: document, section, figure, date). Ratings that compatibility checks will use cannot be saved without a citation ([`compatibility-checks.md`](compatibility-checks.md)).
- **Model:** bind a built-in analog kind, attach an unencrypted SPICE file, or (later) a Verilog-A source. The pin map is visible next to the `.subckt` port list. A “Check model” action runs the checks in section 8.
- **Variants and BOM:** one or more `[[variant]]` rows with MPN, manufacturer, footprint, and optional offers. Official parts need at least one orderable variant, except for bench instruments, which stay out of the BOM as they do today.
- **Licence:** `data`, `artwork`, and a new model-licence record (see section 10). The official-contribution path requires CC-BY-SA-4.0 for data and artwork.

Saving writes the folder in canonical form, suitable for `git diff` and for `ohmforge validate-parts`.

### 6.3 Import and export

- **Import SPICE:** choose a `.lib` / `.cir` / `.mod` file; the tool lists subcircuits and models, refuses encrypted content, and asks the user to confirm the licence before it copies or references the file (section 8 and [`model-and-waveform-interop.md`](model-and-waveform-interop.md) §3.2).
- **Import KiCad symbol or footprint** (P4): read a documented KiCad symbol/footprint library, map pins by number or name, and say what did not map.
- **Import from a datasheet** (P4): reuse the planned datasheet extraction (D3) to propose part name, pin table and ratings with page citations. Nothing is written until the user accepts each field.
- **Export a part pack:** the folder, or a zip of that folder, with a short manifest (`part` id, format version, licences). This is a *part* pack, not a project pack ([`examples-and-sharing.md`](examples-and-sharing.md)).
- **Export KiCad symbol / footprint** (P4): emit a small library the user can add to KiCad. Circuit-level KiCad export remains in [`kicad-export.md`](kicad-export.md); it should consume the same symbol mapping.
- **Export an ngspice-runnable testbench** (P3): a deck plus the bound models, for the user to run in their own ngspice.

### 6.4 In Ohmforge versus standalone

In the Ohmforge desktop app the creator is a window (or a mode) reached from the parts library: “New part”, “Edit official part” (working copy), “Import part pack”. It does not require a running firmware simulation. The standalone build is the same UI and CLI without the SoC, demo picker or instruments. Both write the same folder format.

## 7. Data model mapping

The creator writes ordinary part folders. Planned additions are schema extensions, not a second format. Names below are indicative; E6 and a small RFC may pin them.

| Creator field | `part.toml` / pack file | Notes |
|---|---|---|
| Identity | `[part]` `id`, `name`, `category`, `datasheet`, `model`, `verification` | Ids stay stable; renaming is a new id plus a recorded alias if we ever need one |
| Pins | `[[pin]]` `number`, `name`, `kind` | All pins, including supply and ground; no hidden ground |
| Pin order for the model | `[analog].pins` | Must match the subcircuit port order or a named map |
| Ratings | `[electrical]` tables with `min`/`typ`/`max`, `unit`, citation | Structured citation fields may grow beside `note` |
| Analog kind or subcircuit | `[analog]` `kind`, `file`, `subckt`, parameters | Existing `subckt` / `resistor` / `led` / … kinds |
| Model source record | `[model_source]` or `[license].model` (*planned*) | URL, checksum, retrieved date, licence, redistributable flag |
| Variants | `[[variant]]` | MPN, manufacturer, footprint, offers |
| Symbol | `[symbol]` and/or `symbol.svg` (E6) | One official graphic language, documented |
| Breadboard artwork | `breadboard.svg` or equivalent (E6) | True pitch; used by the P3 breadboard view |
| Licences | `[license]` `data`, `artwork`, plus model record | Official tree: CC-BY-SA-4.0 for data and artwork |
| Tests | `tests/*.toml` or a Rust test next to the part (*planned* pack-local test) | Expected readings, ngspice deck path, cited figure |

Instrument profiles (`parts/instruments/…/instrument.toml`) stay a separate schema; the creator may later grow an instrument mode, but it is not required for C1–C3.

## 8. Validation, import and ngspice checks

A part is not “done” when the TOML parses. The creator and `ohmforge validate-parts` (extended, not replaced) share the same checks:

1. **Schema and invariants.** Known kinds, required fields, pin names unique, analog pin list refers to declared pins, `subckt` has `file` and `subckt`, official IDs match the folder convention.
2. **Citations.** Any rating that a compatibility check or a datasheet test uses has a source. Missing citations are errors for official contributions and warnings for personal packs.
3. **Licence.** `data` and `artwork` present. If a model file is copied into the pack, its licence is present and allows redistribution. If it does not, the tool stores a reference only.
4. **Encrypted models.** If a file looks encrypted or is wrapped in a vendor encryption container, refuse it. No decrypt path.
5. **Pin map versus model.** For a SPICE subcircuit, every port is mapped and the count matches; unused ports must be explicit.
6. **Model versus cited figures.** Reuse the style of the NE555 and AMS1117 datasheet checks: a small fixture, expected values and tolerances taken from `[electrical]`. The creator can run these as “Check model”.
7. **ngspice cross-check, out of process.** Where the model is a SPICE subcircuit or a deck Ohmforge can export, generate a testbench and, if ngspice is installed, run it as an external program and compare selected nodes within a documented tolerance (the same 1 % spirit as the solver goldens, with per-test exceptions recorded). If ngspice is missing, the Ohmforge-side checks still run and the report says the oracle was skipped.
8. **Artwork.** Symbol pins coincide with the pin table; breadboard pitch is 2.54 mm; exported KiCad symbols (P4) pass a syntax check.

CI for an official part PR runs (1)–(5) always, (6) when tests exist, (7) when ngspice is available in the job, and renders symbol and breadboard previews in the PR as already sketched in §13.4.

## 9. Interop formats

| Direction | Format | Phase | Licence note |
|---|---|---|---|
| In | Unencrypted SPICE `.lib` / `.subckt` / `.model` | P3 | Copy only when redistribution is allowed; otherwise reference |
| Out | ngspice-runnable deck | P3 | Export of *our* netlist view; not a redistribution of a forbidden model |
| Out | Analog waveform from a check run (CSV, VCD, later SPICE raw) | P3 | Same as [`model-and-waveform-interop.md`](model-and-waveform-interop.md) §3.3 |
| In / out | KiCad symbol and footprint (pinned format) | P4 | Generate and parse documented file formats; do not vendor KiCad source or link GPL code |
| In | Datasheet PDF (user-cached) | P4 | Never committed; extraction is a proposal |
| In | Verilog-A source, compiled by an OpenVAF sidecar | P5 | Compiler out of process; generated objects need a licence review |
| In | IBIS pin models | P5 | Bound to pins; same reference-or-copy rule as SPICE |
| In | HDL as a digital block, out of process | P6 | Optional; not the default community-part path (WASM is) |

## 10. Licensing of user-made parts

- **Official library** (`parts/` in this repository): data and artwork CC-BY-SA-4.0; code models in `of-components` AGPL-3.0-or-later. A pull request that does not meet that bar is not merged.
- **Personal and third-party packs:** the user may choose another licence, but the pack must declare it. Ohmforge will load a pack only when the licence is declared and the model files in it are either unencrypted and redistributable, or referenced and fetched locally. We do not silently relicense someone else's model as CC-BY-SA-4.0.
- **Vendor models:** follow `DESIGN_AND_PLAN.md` §7 and the interop note: restrictive terms mean URL + checksum + fetch by the user, never a committed file. Encrypted files are refused.
- **Standalone use:** the tool itself stays AGPL-3.0-or-later, like the rest of the code. Packs it writes are data: they take the licence the user (or the official policy) assigns, not automatically the AGPL.
- **Attribution:** imported KiCad or SPICE content keeps its original notices in the pack (a `NOTICE` or fields on `[model_source]`).

## 11. Standalone versus in-app packaging

- **C1–C2:** CLI in `of-cli` (`ohmforge part …`) and a desktop window in `of-app`. Shared logic lives in a workspace crate (working name `of-part-creator` or an extension of `of-parts`). AGPL-3.0-or-later.
- **C5 (P6):** a standalone binary (and optionally a smaller desktop crate) that depends on `of-parts`, the validator, the SPICE import/export path and the symbol editor, and does *not* link the SoC, the kernel or the demo set. One codebase, feature-gated or a thin entry crate. This is what makes the tool usable as a general part authoring utility.
- Installation: desktop Ohmforge can offer “Also install the part creator command”. A later standalone installer is a packaging decision, not a second source tree.
- The web app (P4) is not required to host the symbol editor in 0.3; C5 may consider a wasm build of the creator once `of-web` exists, but it is not a 0.2 requirement.

## 12. Phasing and acceptance

### C1 — CLI, schema and model hygiene (P3 / 0.2)

Depends on E6 for `[symbol]` and `bom` fields, or lands a documented subset if E6 is only partly ready (pins, electrical, analog, licence and `model_source` can ship first).

- `ohmforge part new` creates a valid skeleton pack.
- `ohmforge part import-spice` and `ohmforge part export-spice` (names *planned*) implement the unencrypted import, licence record, encrypted-file refusal, and ngspice-runnable deck from [`model-and-waveform-interop.md`](model-and-waveform-interop.md) §3.2.
- `ohmforge validate-parts` runs the checks in section 8 that do not need a GUI.
- At least one official part (for example a new regulator or comparator already on the P3 macromodel list) is authored or re-checked with the CLI, including an ngspice testbench when ngspice is present.

Acceptance: a contributor can add a subcircuit part without hand-inventing licence or pin-map conventions; CI fails a pack that is missing a licence, a pin map mismatch, or an encrypted model file.

### C2 — Desktop authoring UI (P3 / 0.2)

- A window to edit identity, pins, ratings, variants, licence, and a first symbol editor (basic primitives, pin placement, preview).
- Bind an existing `[analog]` kind or an imported subcircuit; “Check model” runs the C1 checks and shows failures next to the field.
- Save a pack that `validate-parts` accepts. Official NE555 or AMS1117 can be opened read-only or as a working copy and saved without unintended diffs (canonical formatting).

Acceptance: a new contributor authors a resistor-class or subcircuit part in the UI, validates it, and uses it from a hand-written circuit file under `ohmforge run` (full in-app placement still needs the circuit editor and E6).

### C3 — KiCad and datasheet assistance (P4 / 0.3)

- Import and export of a pinned KiCad symbol (and footprint if the schema is ready), shared with the circuit exporter's mapping tables.
- Datasheet-assisted fill using D3: proposed pins and ratings with page citations, accepted field by field.

Acceptance: a KiCad symbol for a dual op-amp imports, pins map, and the exported symbol opens in the pinned KiCad version; a datasheet proposal never writes a field the user did not accept.

### C4 — Compact models and pin models (P5 / 0.4)

- Bind a Verilog-A source through the OpenVAF sidecar, and an IBIS model to pins, with the same licence rules.
- Optional use of the ideal-switch mode in a converter part's test fixture, labelled as such.

Acceptance: one Verilog-A and one IBIS example pack, each with a failing path when the sidecar or file is missing, and no GPL/LGPL linkage.

### C5 — Standalone tool and authoring guide (P6 / 1.0)

- Standalone binary (or a clearly documented feature set) that creates, validates, imports and exports packs without starting a simulation.
- The P6 part-authoring guide is written against this tool, not against a hidden hand-edit ritual. `CONTRIBUTING.md` points here.

Acceptance: a person who does not want the ESP32 simulator can install the part creator, make a pack, export a KiCad symbol, and validate it; the user guide describes that path.

## 13. Risks

| Risk | Mitigation |
|---|---|
| Scope becomes a second EDA symbol editor and never ships | C2 primitives are deliberately small; KiCad remains the rich symbol tool, with import/export in C3 |
| Licence accidents with vendor models | Refuse encrypted files; require a licence record; official CI rejects committed restrictive models |
| Schema churn with E6 | Creator writes the same `part.toml`; symbol fields follow E6 rather than a parallel format |
| Canonicalisation noise in git | One official formatter (`part fmt`, or extend `validate-parts --write`) and goldens for official packs |
| Datasheet extraction invents figures | Proposals only; citations required; official parts still need human review |
| Standalone packaging forks the code | Feature gates / a thin entry crate, not a second implementation |
| Contributors ignore the tool and keep pasting TOML | CLI is useful on its own; official new parts after C1 are expected to pass the same checks, whether or not the UI was used |

## 14. Open questions

- The exact command names (`ohmforge part new` versus `ohmforge new-part`) and whether “edit official part” always copies into a working tree.
- Symbol storage: SVG, a TOML `[symbol]` of primitives, or both (SVG derived in CI).
- How far C2's symbol editor goes toward IEC 60617 and ANSI/IEEE 315 (the theme work, T3, already plans a default of IEC). A minimum is a unique, labelled body with pins that the schematic view can draw.
- Whether pack-local tests live in TOML (so the standalone tool can run them) or only as Rust tests in `of-components` (stronger, but not standalone).
- The pinned KiCad format version, shared with [`kicad-export.md`](kicad-export.md).
- Whether a personal pack licence other than CC-BY-SA-4.0 can be offered upstream without a relicensing step (likely no: upstream requires CC-BY-SA-4.0, and the contributor must have the right to grant it).
- Whether instrument profiles share this tool in P3 or stay hand-written until after C2.
