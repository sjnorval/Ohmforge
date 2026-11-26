# Part creator

Status: proposed (design only, no code). Expanded 2026-11-26 so a developer can implement the tool without guessing: every subsection is grounded in the current crates and `part.toml` files, and planned schema extensions are marked as such.
Date: 2026-11-26.
Related: the detailed notes under [`part-creator/`](part-creator/); [`model-and-waveform-interop.md`](model-and-waveform-interop.md); [`kicad-export.md`](kicad-export.md); [`compatibility-checks.md`](compatibility-checks.md); [`parts-availability.md`](parts-availability.md); [`engine-components.md`](engine-components.md) E6; [`editor-integration.md`](editor-integration.md); [`CONTRIBUTING.md`](../../CONTRIBUTING.md#adding-a-part); [`parts/README.md`](../../parts/README.md); [`parts/LICENSE.md`](../../parts/LICENSE.md).

The quality and size of the parts library is the main limit on whether common hobby projects actually run in Ohmforge. This note (and the notes it indexes) specify a **part creator**: a tool that authors a complete part pack and, over time, can stand on its own as a general-purpose part authoring tool.

**Read this index, then the sub-notes, before writing code.** The current schema is smaller than the finished tool. Do not invent fields in `of-parts` that contradict `crates/of-parts/src/lib.rs`; extend it with a documented schema version.

## Contents

1. [Data model](part-creator/data-model.md) — every `part.toml` field the code actually parses today, planned extensions, pins, variants, IDs, versioning.
2. [Units and numbers](part-creator/units-and-numbers.md) — SI suffixes, `1M` versus `1meg`, micro signs, locale commas, tolerances, float precision.
3. [SPICE import](part-creator/spice-import.md) — what `of-netlist` accepts, `.lib` and behavioural sources, encrypted-file refusal, licence records, pin-order traps.
4. [Verification](part-creator/verification.md) — ngspice out of process, 1 % of span, datasheet tests, CI, what to do when results diverge.
5. [Ratings, citations, symbols and BOM](part-creator/ratings-symbols-bom.md) — citation shape, abs-max versus recommended, symbol/KiCad pitfalls, MPN and ZAR prices.
6. [Licensing](part-creator/licensing.md) — official CC-BY-SA packs, user packs, provenance, KiCad and vendor-model rules.
7. [UX and testing](part-creator/ux-and-testing.md) — flows, undo, autosave, errors, tests per milestone, acceptance and definition of done for C1–C5.
8. [Pitfalls, checks, risks and open decisions](part-creator/pitfalls-and-decisions.md) — numbered checklist (detection, mitigation, test), risk register, decisions that need a maintainer call.

## 1. Why this, and not another plan item

The 2026-11-26 audit added several simulator techniques to the plan. Those are worth doing. The question is which one most improves Ohmforge if we specify it properly now.

- **Always-on simulation in the editor** is an important feel for the schematic editor, but it is blocked on that editor and does not grow the library.
- **SPICE import, waveform export, Verilog-A, IBIS, a converter switch mode and HDL sidecars** make models richer. They pay off only when someone can bind a model to a real, orderable part. The first of those belong *inside* the part creator.
- **Release 0.2 aims at about 50 cited, tested parts** ([`COMPONENTS.md`](../COMPONENTS.md)). Engine work on `main` is already ahead of the library: E4 is done, E5 is all but follow-ups, E6 is still open.
- **`DESIGN_AND_PLAN.md` §13.4** already sketches `ohmforge new-part` creating `part.toml`, `symbol.svg` and `breadboard.svg`. This design makes that real.

The part creator is therefore the largest win. It is not a replacement for the circuit schematic editor, and it is not the parked in-app firmware editor.

## 2. Users

- **Ohmforge maintainers and contributors** adding official parts to `parts/` (CC-BY-SA-4.0 data and artwork).
- **Project authors** who need a part that is not in the official library yet.
- **People outside Ohmforge** (a later audience) who want a cited, simulation-ready pack they can export to KiCad or keep as a folder. This path must not depend on an ESP32 being in the circuit.

## 3. Current state (code, not plans)

Ground truth, as of `main` through PR #99:

| Piece | Where it lives | What it actually does |
|---|---|---|
| `part.toml` schema and loader | `crates/of-parts/src/lib.rs` | `parse_part`, `load_dir`, `PartFile::check` |
| Official library | `parts/**/part.toml` | One folder per part; `parts/LICENSE.md` |
| Analog bridge | `crates/of-components/src/analog.rs` | Maps `[analog]` kinds onto `of-analog` devices; `to_spice`; loads `file` next to `part.toml` |
| SPICE subset | `crates/of-netlist/src/spice.rs`, `expr.rs` | Parser used by `ohmforge run` and by the bridge for `subckt` parts |
| CLI validation | `crates/of-cli/src/main.rs` (`validate_parts`) | Parse, `check()`, require `license.data` and `artwork` to be `CC-BY-SA-4.0` |
| BOM v1 | `crates/of-bom/src/lib.rs` | First `[[variant]]` only; `price_zar_cents`; `ohmforge bom` |
| Datasheet tests | `crates/of-components/tests/ne555_datasheet.rs`, `ams1117_datasheet.rs` | Cited figure checks through the bridge |
| ngspice goldens | `scripts/gen-ngspice-golden.sh`, `crates/of-analog/tests/ngspice_compare.rs` | External ngspice; 1 % of span (see the verification note) |
| Circuit files | `crates/of-circuit`, RFC 0003 | Pins are resolved against the parts library |
| KiCad export | `of-kicad` crate is a placeholder; [`kicad-export.md`](kicad-export.md) | Not implemented; footprints today are name strings only |
| `[symbol]`, breadboard art, `bom` flag | Planned in E6 | Not in `PartFile` yet |

Limits of today's validator, which the creator must not silently inherit: `PartFile::check` does not require unique part ids, does not check that `analog.file` exists, does not validate `verification` or pin `kind` vocabulary, and does not compare `[electrical]` to the model. The unit test `every_library_part_parses_and_checks` in `of-parts` *does* check unique ids, CC-BY-SA-4.0 and that `analog.file` is present on disk; `ohmforge validate-parts` does not. Offers have no date. `[electrical]` is an untyped `BTreeMap<String, toml::Value>`.

## 4. Goals, non-goals and packaging

Goals, non-goals, crate placement (`of-part-creator` or an extension of `of-parts`), CLI versus desktop versus standalone, and the C1–C5 phase list are unchanged in intent from the first draft; the detailed requirements now live in the sub-notes. In short:

- C1 (P3): CLI, schema hygiene, unencrypted SPICE import, ngspice-runnable deck, a stricter validator.
- C2 (P3): desktop authoring UI.
- C3 (P4): KiCad import/export and datasheet-assisted fill.
- C4 (P5): Verilog-A and IBIS bindings.
- C5 (P6): standalone binary and the part-authoring guide.

Non-goals still include a circuit editor, a firmware editor, auto-writing models from a datasheet with no review, vendoring third-party symbol libraries into `parts/`, decrypting models, and linking GPL or LGPL tools.

## 5. Definition of done (for the design)

This design is ready for implementation when a developer can answer, from the sub-notes and the cited code, what to parse, what to write, what to refuse, what to test, and which questions are still waiting on a maintainer decision. Those open decisions are collected in [`part-creator/pitfalls-and-decisions.md`](part-creator/pitfalls-and-decisions.md) §3.
