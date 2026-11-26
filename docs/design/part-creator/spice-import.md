# Part creator: SPICE import

Status: proposed (design only). Grounded in `crates/of-netlist/src/spice.rs`, `crates/of-netlist/src/expr.rs`, and `of_components::analog` (`to_spice` and `kind = "subckt"`).
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md). Related: [`../../model-and-waveform-interop.md`](../../model-and-waveform-interop.md) §3.2.

The importer turns a public, unencrypted SPICE library into a `part.toml` binding (`kind = "subckt"`, `file`, `subckt`, `[analog].pins` in port order) plus a licence record. It uses the same parser the simulator uses. If Ohmforge cannot parse the file, it must say so and must not pretend the official solver will run it.

## 1. What the parser actually accepts

From the rustdoc at the top of `spice.rs`, the supported subset is **ngspice-compatible for Ohmforge’s own tests and imports**:

- Elements: `R C L K V I D Q M E G F H S W X`.
- `.model`, `.subckt`/`.ends` (flattened on parse, with `params:` and subcircuit-local `.param`/`.model`).
- `.param` expressions (`expr.rs`).
- `.op`, `.dc` (one or two sources), `.ac`, `.tran`, `.temp`, `.ic`, `.options`, `.include` / `.inc`, `.end`.
- `.control` blocks and output directives (`.print`, `.save`, `.meas`, …) are skipped so the same file can still run in ngspice.
- Everything is **case-insensitive** and stored lower-case.
- Continuation lines start with `+`.
- Comments: a line starting with `*`, and inline `;`. A ` $` or tab-`$` also starts an inline comment (`strip_comment`).
- `.include` is resolved relative to the current file. Nesting is depth-limited.
- Nested `.subckt` *definitions* are rejected. Recursive instantiation is depth-limited.

## 2. What will fail or surprise

These are implementation facts, not hypotheticals:

- **`.lib` is not supported.** An unknown directive is a hard error (`unsupported directive {other}`). Vendor files often wrap models in `.lib typ` / `.lib min` sections. C1 must detect `.lib`, explain that Ohmforge currently understands `.include` of a file that already contains `.subckt`/`.model`, and either refuse or (a planned follow-up) expand a chosen section into a temporary included file. Do not silently skip the card.
- **Behavioural `B` sources** (and other letters outside the list above) are `unsupported element`. Many modern macromodels use them. Refuse with the parser’s line number, and suggest a primitive or an in-tree macromodel instead of a silent stub.
- **MOSFET levels other than 1 and 3** fail in `of-analog` netlist build (`MOSFET level {level} is not supported yet`). A library full of level-49 or BSIM models will not run as primitives; they may still be wrapped only if the solver grows that model, which it has not. Import can still *record* the file, but “Check model” must fail clearly.
- **`.noise`, `.tf`, `.sens`, …** produce a warning and are not run (`spice.rs`).
- **Nested `.subckt` definitions** are unsupported; flatten or refuse.
- **Default `params:`** on `.subckt` and instance `X … params:` are supported. Instance parameters from `[analog].params` are appended as `k=v` on the `X` card in `analog.rs`. Unknown names may be ignored or fail later; the creator should list the subcircuit’s default param names and write only those.
- **`.include` paths** in a vendor file are often relative to the vendor’s tree. After a copy into the part folder, those paths break. The importer must rewrite includes to files it actually copies, or refuse the include with the missing path. `scripts/gen-ngspice-golden.sh` already rewrites relative `.include` to absolute paths for a similar reason.
- **Line endings and encoding:** parse UTF-8 / ASCII. A UTF-16 dump, or a file with a BOM-only first line, should be a clear error.
- **Case:** names are lower-cased. A subcircuit `NE555` is stored as `ne555`. The `subckt` field in `part.toml` should use the lower-case name Ohmforge will look up, and the creator should show both the original spelling and the stored one.

## 3. Pin order

`[analog].pins` is “part pins (numbers or names) in device terminal order … or the subcircuit’s port order” (`AnalogModel` rustdoc). The bridge builds `Xname n1 n2 … subckt` from those nets in list order (`analog.rs`).

If the symbol or the package lists pins as 1=GND … 8=VCC, but the `.subckt` line is `.subckt ne555 GND TRIG OUT RESET CONT THRES DISCH VCC` (or some other order), the analog pin list must follow the **`.subckt` line**, not the datasheet pin-1-first table. `ti.ne555` does this correctly: `pins = ["GND", "TRIG", "OUT", "RESET", "CONT", "THRES", "DISCH", "VCC"]` matching the eight `[[pin]]` names.

The importer UI must show three columns side by side: package number, name, and subcircuit port index. Saving is blocked until every port is mapped and the counts match. Unused ports must be explicit (tied in the model or marked unused), never dropped.

## 4. Encrypted and non-redistributable models

There is no detector in the tree today. C1 must add one and **refuse** these files: no decrypt path, no “import anyway” checkbox for official packs.

Detect (any one is enough to refuse):

- The word `encrypted` (any case) in the first few kilobytes of text, or a header line that says the body is a ciphertext payload.
- Long runs of high-entropy printable text with no parseable element cards, `.model`, or `.subckt`.
- NULs or a large fraction of non-text bytes (binary).
- Known wrapper directives and markers used by commercial encryption containers, kept in a table we maintain (pattern, source). Do not try to unwrap them.

On refusal, show the path, the rule that matched, and the instruction: use a public unencrypted model, or record a URL + checksum + licence for a user-side fetch, as `DESIGN_AND_PLAN.md` §7 already requires. Never write an encrypted body into `parts/` or into a pack that will be committed.

For unencrypted files whose licence forbids redistribution: do not copy the file. Write `[model_source]` (planned) with URL, SHA-256, date, licence, `redistributable = false`, and a fetch hint. Official CI must fail if such a file is present in the pack.

## 5. Licence and source capture

For every imported file the user *does* copy:

- Ask for the licence (or “unknown — do not copy into the official tree”).
- Store URL, retrieval date, SHA-256 of the exact bytes, licence, and `redistributable`.
- Keep a `NOTICE` or field for the original copyright line.
- Re-hash on “Check model” and on official CI. A silent edit of the `.lib` is a fail.

Our own macromodels (`ne555.lib`, `ams1117.lib`) are original work under CC-BY-SA-4.0, as their `part.toml` comments say. The importer must not pretend a vendor file is original work.

## 6. Export of an ngspice-runnable deck

`to_spice` in `analog.rs` already emits cards for a board: primitives, `.model` cards, and each subcircuit library once. LED cards use Ohmforge’s `led` model type and capacitors may use `esr=`/`esl=`. The golden script’s `* ngspice-replace:` lines exist because not every Ohmforge card is plain ngspice.

The C1 exporter must:

- Write a self-contained deck (library body + a small test fixture + `.tran`/`.op` as requested).
- List devices and options that ngspice will not accept unchanged (LED type, ESR syntax, unsupported elements) in a log, and apply the same class of replacements the golden script uses, or refuse to claim “ngspice-runnable”.
- Use `1e6`/`1meg`, never `1M`.
- Run ngspice only as an **external program**, the way `scripts/gen-ngspice-golden.sh` already documents (ngspice is never linked). If `ngspice` is missing, the Ohmforge-side checks still run and the report says the oracle was skipped.

## 7. Convergence-hostile models

Some vendor macromodels are stiff, use discontinuities, or need specific `.options`. “Check model” should:

- Run with the same default options as `ohmforge run`.
- On non-convergence, report the solver error and, if ngspice was run, its log tail. Do not auto-tweak tolerances to force a pass.
- Allow a documented per-test option overlay only when a comment in the pack explains why, and never as a silent official default that hides a bad model.
