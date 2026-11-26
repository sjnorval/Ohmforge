# Part creator: UX, testing, milestones and definition of done

Status: proposed (design only).
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

## 1. User-facing flows

Flows already sketched in the index (new part, edit panes, import SPICE, export deck / later KiCad, official versus personal) stand. Implementation rules:

- **One pack at a time.** The window is a part editor, not a library browser with multi-document chaos. A library picker can open a different pack after a dirty-file prompt.
- **Undo / redo.** Every field edit, pin reorder, and import mapping step is undoable. Import of a file is one undo group (“Import SPICE”) so a bad import is one shortcut away from the previous pack, not a hundred token edits. C2 must have undo before it is called done; C1 CLI does not need it.
- **Autosave.** Autosave to a sibling `*.autosave` (or a user cache), not to the official file, every few seconds while dirty. Recovery offers “Restore autosave” on next open. Never autosave over a clean git checkout of `parts/`.
- **Validation as you type (C2).** The same checks as `PartFile::check` plus official/personal rules run on a debounce (about 300 ms) and on save. The pin table, the subcircuit port map, and the licence row show inline errors. Save of an official pack with errors is refused; a personal pack may save with warnings but not with parse errors or an encrypted model.
- **Error messages.** Use the same voice as the CLI today (`FAIL id (path)` then `- problem`). Include file, field, and, for SPICE, the parser’s `line N: message` from `ParseError`. Suffix mistakes must show the parsed SI value (`1M` → 0.001 milli; did you mean 1meg?). Missing `analog.file` must name the expected path. Licence errors must name official versus personal mode.
- **Check model** is a visible button (and `ohmforge part check`). It produces a report the user can save (TOML or Markdown): steps run, bands used, ngspice version or “oracle skipped”, and decks written to a temp or `check/` folder.
- **Read-only official parts.** Editing `parts/ti.ne555` in a dirty checkout is allowed; the UI should say “you are editing the official library” and offer “copy to a personal pack” as the default suggestion for experiments.

Keyboard: tab through the pin table, escape cancels a mapping dialog, and the CLI remains usable without the GUI (C1 can land first).

## 2. Testing strategy per milestone

### C1 — CLI and hygiene

- Unit tests on any new `model_source`, format field, official/personal validator, and encrypted-file detector (fixtures: a tiny `.subckt`, a file containing an `encrypted` header, a file with NULs, a `.lib` section file, a `B` source).
- Property / fuzz tests for `parse_value` and for the importer’s tokenisation: random suffix strings should either parse to the documented scale or return an error, never a silent wrong magnitude. Include `1M`, `1m`, `1meg`, `1mil`, `1μ`, `1,0`, `10uF`.
- Round-trip: `part new` → fill required fields → write → `parse_part` → `part fmt` → byte-identical second fmt. Official fixtures include `ti.ne555` and `generic.resistor` opened and saved without unintended diffs (canonicalisation must not churn licence comments or pin order).
- Golden decks: one primitive and one subcircuit export checked as text (or parsed), plus a test that `1e6` appears and `1M` does not.
- `validate-parts` tests: unique id clash, missing `.lib` file, bad pin map, personal pack with a non-CC-BY-SA licence (fails official mode, passes personal mode if declared), encrypted file always fails.

### C2 — UI

- Headless or egui tests for: undo of a pin reorder, autosave does not overwrite a clean file, validation messages appear for a known-bad pack, “Check model” shows “oracle skipped” when ngspice is absent.
- Manual acceptance: author a resistor-class part and a subcircuit part, validate, `ohmforge run` a hand-written circuit file that uses it.

### C3 — KiCad and datasheets

- Golden KiCad symbol text for a one-unit part; pin numbers match; hidden power pins became explicit.
- A dual-unit import is refused or flattened only with the documented warning (until multi-unit is decided).
- Datasheet proposals never write a field without an accept event (unit test on the proposal API).

### C4–C5

- Missing sidecar is a hard, clear failure (no link of a GPL compiler).
- Standalone binary crate graph does not depend on `of-soc-esp32c3` or the demo set (a `cargo tree` or deny rule in CI).

## 3. Acceptance criteria (C1–C5)

These refine, not replace, the phase list in the index.

**C1.** A contributor can add a `subckt` part without inventing licence or pin-map conventions. `ohmforge part new`, `import-spice`, `export-spice`, and `validate-parts` (official and personal modes) exist. Encrypted files are refused. An unencrypted public subcircuit imports, maps ports, validates, and, when ngspice is present, compares a fixture within the 1 %-of-span rule or a tighter cited band. Official CI fails missing licences, id clashes, pin-map mismatches, missing `analog.file`, and encrypted bodies. At least one official part is re-checked with the CLI (NE555 or a new P3 macromodel).

**C2.** A window edits identity, pins, ratings, variants, licence, and (once E6 allows) a basic symbol. Undo and autosave work as above. “Check model” surfaces the C1 report next to the field. Saving `ti.ne555` or `ams.ams1117-3.3` as a working copy does not introduce a noisy diff. A new contributor authors a part in the UI and uses it from a hand-written circuit file under `ohmforge run`.

**C3.** A pinned KiCad symbol for a single-unit part imports and exports and opens in the pinned KiCad version; missing footprints are listed. Datasheet-assisted fill is proposal-only. Official KiCad libraries are the name authority; Tabula and WebPlotDigitizer are user-run helpers as in [`sources.md`](sources.md) and [`../ecosystem-integrations.md`](../ecosystem-integrations.md).

**C4.** One Verilog-A and one IBIS example pack, each with a failing path when the sidecar or file is missing; `deny.toml` still has no GPL/LGPL dependencies.

**C5.** A documented standalone build creates, validates, imports and exports packs without starting a simulation. The P6 part-authoring guide is written against this tool. `CONTRIBUTING.md` points here.

## 4. Definition of done (implementation)

A milestone is done when:

- The acceptance list above for that slice is ticked with tests or a short recorded manual script.
- `AGENTS.md` / `docs/ROADMAP.md` boxes match reality (no “planned” items left marked done, and no landed behaviour left undocumented).
- Licence rules in `parts/LICENSE.md` still hold; official packs in the PR would pass official `validate-parts`.
- Open decisions in [`pitfalls-and-decisions.md`](pitfalls-and-decisions.md) that the slice depends on have a written answer in the note, not a guess in the code.
