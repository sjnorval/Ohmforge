# Part creator: verification

Status: proposed (design only). Grounded in `crates/of-analog/tests/ngspice_compare.rs`, `scripts/gen-ngspice-golden.sh`, `crates/of-components/tests/ne555_datasheet.rs` and `ams1117_datasheet.rs`.
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

A part is not done when TOML parses. Official parts on `main` already use two styles of check; the creator must offer both and must not invent a third tolerance rule without recording it.

## 1. Two kinds of check

**A. Datasheet / behaviour checks** (in-process, Ohmforge only). Pattern: `crates/of-components/tests/ne555_datasheet.rs`. The NE555 comment cites TI SLFS022K and an older revision for timing tolerances, then asserts monostable width to 1 % of `1.1 RC`, astable frequency to 2.25 % of the datasheet formula, and several DC figures with tighter relative bands (0.5 % to 5 % depending on the figure). These checks do not need ngspice.

**B. Oracle checks** (ngspice out of process). Pattern: `scripts/gen-ngspice-golden.sh` produces `crates/of-analog/tests/golden/*.csv`; `ngspice_compare.rs` compares Ohmforge to those goldens. The rustdoc and `LIMIT_PERCENT = 1.0` state the rule:

- Transient and DC sweep signals: within **1 % of the reference signal’s span**, with a floor of `max(1e-3 * max|ref|, 1e-12)` so a flat signal does not divide by zero.
- Operating points: 1 % of the reference magnitude (floor `1e-12`).
- AC magnitudes: point-by-point relative to the reference, with a floor of `1e-3` of the largest magnitude.
- AC phases: as a percentage of 180°.
- Nested DC sweeps: row by row (no interpolation). Other sweeps interpolate Ohmforge onto the golden abscissa (`interpolate`).

The golden script runs `ngspice -b`, pins `option numdgt=12`, `units=degrees`, and records the ngspice version in a header comment. Goldens are committed; they are not regenerated in ordinary CI. Regeneration is a reviewed, deliberate step.

## 2. What the creator should run

“Check model” (C1 CLI and C2 UI) runs, in order:

1. `parse_part` + `PartFile::check` + official/personal licence rules + `analog.file` exists + unique id in the library being validated.
2. Pack-local datasheet fixtures, if present (expected readings, conditions, relative/absolute band). Until pack-local TOML tests exist (open decision), official parts keep Rust tests next to `of-components` as they do today, and the CLI can invoke those by part id when they exist.
3. If the model is a SPICE subcircuit or an exportable deck, and `ngspice` is on `PATH` (or `$NGSPICE`), export the fixture and run it externally. Compare selected nodes with the **same 1 %-of-span rule** unless the pack records a tighter, cited band (datasheet tests may be tighter; they must not be looser than 1 % of span without a written exception).
4. If ngspice is missing, report `oracle skipped` and still return a pass/fail from steps 1–2. Official CI should run the oracle job where ngspice can be installed; a skip on a developer laptop is allowed.

Determinism:

- Use the same timestep / analysis statements in the Ohmforge run and the ngspice deck.
- No wall-clock seeds. The existing engine is deterministic given the same circuit and options.
- Record the ngspice version in the report. A divergence after a version change may be an oracle change, not an Ohmforge regression; treat it like a golden update (reviewed diff, not a silent rewrite).

## 3. When results diverge

A documented playbook, not an improvisation:

1. Re-run both sides with the exact decks saved next to the report (`ohmforge` CSV/VCD and the ngspice log + `wrdata` file).
2. Check for export replacements (LED model, `esr`/`esl`, `1M` suffixes, missing `.include`).
3. If Ohmforge and ngspice disagree by more than the band on a *new* official model, the part does not merge. Either fix the macromodel, shrink the claim (note the limit in `[analog].note`), or add a reviewed, named exception with the same honesty as RFC 0003’s accepted parity exceptions.
4. If they disagree on a *solver* device (primitives already in the 1 % suite), that is an `of-analog` bug, not a part-authoring waiver.
5. Never raise `LIMIT_PERCENT` globally to make a part pass.

## 4. CI

- `cargo test -p of-parts` already parses every `part.toml`, checks unique ids, CC-BY-SA, `check()`, and that `analog.file` exists. Keep this.
- `ohmforge validate-parts parts/` must grow the extra official rules (section 1 of the data-model note) and run in CI. Personal packs use a flag so CI on `parts/` stays strict.
- Datasheet tests stay in `of-components` (or move to pack-local tests that CI also runs).
- `ngspice_compare` stays the solver oracle. Part-level ngspice fixtures, once they exist, are a separate test binary or `ohmforge part check --oracle` job that may be skipped if ngspice is not installed, with the skip visible in the log.
- CI must not call `gen-ngspice-golden.sh` as a write step. Updating goldens is a human-reviewed PR.

## 5. Firmware / SoC parts

`espressif.esp32-c3` has no `[analog]` section; its behaviour is a Rust component. “Check model” for such parts is not a SPICE deck. C1 can still validate pins, licence, variants, and, where tests exist, existing SoC tests (`crates/of-soc-esp32c3/tests/`). The creator should not pretend to emit a `.subckt` for the MCU.
