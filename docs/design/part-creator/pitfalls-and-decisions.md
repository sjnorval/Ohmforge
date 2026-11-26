# Part creator: pitfalls, checks, risks and open decisions

Status: proposed (design only).
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

## 1. Pitfalls and checks

Each item: what goes wrong, how we detect it, how we mitigate it, and how we test it. Implement these as validator rules or importer checks unless a row says “UI only”.

1. **`1M` is milli, not mega.** `parse_value` lowercases the token, so `1M` becomes `1m` = 10^{-3}; mega is `1meg` (`crates/of-netlist/src/spice.rs`). **Detect:** any imported or typed suffix `M` / `m` shows the parsed SI value. **Mitigate:** warn and ask; never write `1M` in exported decks (use `1e6` or `1meg`). **Test:** fuzz/unit cases `1M`, `1m`, `1meg`, `1mil`.

2. **Micro sign and locale commas.** `1μF` / `1µF` are not ASCII `u`; `1,0` is not a decimal. **Detect:** parse failure in the importer and in the UI number field. **Mitigate:** reject with a hint to use `1u` or `1e-6` and a period decimal. **Test:** those tokens return an error, not a plausible wrong value.

3. **SPICE suffixes written into `part.toml`.** TOML params are raw SI `f64`. **Detect:** a string param where a number is required fails `parse_part`. **Mitigate:** the UI converts display suffixes to SI before write; `part fmt` never emits `1k`. **Test:** round-trip of a 10 kΩ resistor writes `10000.0` or `1e4`, not `10k`.

4. **`tol = 1` meant as 1 %.** `ParamSpec.tol` is relative (`0.01` = 1 %). **Detect:** official validation warns if `tol >= 0.5` unless a note explains it. **Mitigate:** the UI labels the field “relative, 0.01 = 1 %” and previews min/max. **Test:** a fixture with `tol = 1` warns; `tol = 0.01` on the generic resistor does not.

5. **`.lib` section files and `.include` paths.** The parser does not accept `.lib` and resolves `.include` relative to the current file. **Detect:** `unsupported directive .lib`; missing include path. **Mitigate:** clear error, or a later expander for a chosen `.lib` section; rewrite includes when copying a tree. **Test:** a `.lib typ` fixture fails with a message that names `.lib`; a broken include names the path.

6. **Unsupported devices (behavioural `B`, MOSFET levels other than 1 and 3).** **Detect:** parser or netlist-build error with a line number. **Mitigate:** refuse to mark “Check model” as passed; do not stub a primitive silently. **Test:** a `B` source file and a `LEVEL=49` MOSFET fail as documented.

7. **Subcircuit port order ≠ package pin order.** The bridge emits `X` ports in `[analog].pins` list order (`analog.rs`). **Detect:** importer shows three columns (number, name, port index); count mismatch is an error. **Mitigate:** block save until every port is mapped; official CI runs `PartFile::check`. **Test:** swapping two NE555 analog pins makes the monostable test fail; the validator catches an unknown name.

8. **Pin number / name lookup clashes.** `PartFile::pin` matches number exactly, then name case-insensitively. **Detect:** a name equal to another pin’s number, or two names that differ only by case. **Mitigate:** C1 official validation rejects both. **Test:** a fixture with `number = "1"` and another pin `name = "1"` fails.

9. **`ground` is a used kind but not listed in the `Pin` rustdoc.** `espressif.esp32-c3` uses it. **Detect:** vocabulary check allows `power|ground|input|output|io|passive`. **Mitigate:** document the six kinds; update rustdoc when the schema work lands. **Test:** `kind = "ground"` passes; `kind = "pwr"` fails.

10. **Hidden KiCad power pins.** The engine forbids hidden grounds. **Detect:** on import, hidden pins are listed. **Mitigate:** they become explicit `[[pin]]` entries or the import is refused. **Test:** a symbol with a hidden VCC/GND produces two explicit pins.

11. **Encrypted or forbidden vendor models in the tree.** **Detect:** encrypted-file heuristics (spice-import §4); official CI forbids undeclared model files and `redistributable = false` bodies. **Mitigate:** refuse; offer URL + checksum only. **Test:** an `encrypted` header fixture always fails; a NOTICE-only reference pack passes official validation without the forbidden bytes.

12. **`validate-parts` is stricter than the author thinks, in the wrong direction.** It requires CC-BY-SA on *every* pack, but does not check unique ids or missing `analog.file` (those are unit-test-only today). **Detect:** this is a process bug. **Mitigate:** official versus personal modes; promote unique-id and file-exists checks into the official CLI. **Test:** two parts with the same id fail official validation; a missing `.lib` fails; a personal MIT pack fails official and passes `--personal`.

13. **BOM uses the first variant only.** `Bom::from_placements` (`of-bom`). `ti.ne555` would always buy `NE555P` (DIP-8). **Detect:** UI warning when a part has more than one variant. **Mitigate:** default variant is first; document it; E6 should let a placement name the variant (open decision). **Test:** a BOM of a two-variant part matches the first MPN; a future placement field, once it exists, selects the second.

14. **`price_zar_cents` has no date and is easy to mis-scale.** `4590` means R45.90, not 4590 rand. A1 plans millionths. **Detect:** UI preview as `R45.90 (snapshot, not live)`. **Mitigate:** integer cents only, until a dated Money type exists. **Test:** format tests already in `of-bom`; creator preview test for 12 → `R0.12`.

15. **`[electrical]` drifts from the model.** `espressif.esp32-c3` still cites a typ 5 mA load in `[electrical]` while the component uses the datasheet 23 mA / 16 mA figures. **Detect:** “Check model” compares cited supply current to a measured or documented model default when both exist. **Mitigate:** warn; official parts should be updated as a follow-up, not silently ignored. **Test:** a fixture where electrical typ and model default differ by more than 10 % warns.

16. **Generic resistor typ versus demo variant.** `[analog]` typ 1000 Ω, demo variant 330 Ω (`parts/passives/resistor/part.toml`). **Detect:** UI shows “placement overrides the typ”. **Mitigate:** keep typ as a default; variants carry the sold value in the label/MPN. **Test:** documentation example in the new-part template comment.

17. **Canonicalisation churn.** Saving an official part reorders keys or rewrites comments and pollutes git. **Detect:** round-trip goldens for `ti.ne555` and `generic.resistor`. **Mitigate:** `part fmt` is deterministic; comments in the official files are preserved or the format is accepted as canonical with a one-shot reformat PR. **Test:** second fmt is byte-identical.

18. **ngspice and Ohmforge decks are not identical.** `to_spice` may emit LED / ESR syntax that the golden script rewrites. **Detect:** exporter log of replacements; “Check model” fails rather than claims ngspice-runnable if replacements are not applied. **Mitigate:** reuse the `* ngspice-replace:` idea or a maintained mapping table. **Test:** a LED fixture export either contains a documented replacement or the log lists it.

19. **Oracle skipped treated as a pass in CI.** **Detect:** the report line `oracle skipped`. **Mitigate:** allowed on a laptop; official oracle job is visible; a skip in the ngspice CI job is a fail if ngspice was supposed to be installed. **Test:** CLI exit code documentation; CI script asserts ngspice is present in the oracle job.

20. **Non-convergence “fixed” by loosening tolerances.** **Detect:** pack options overlays must be named and commented. **Mitigate:** no silent global change to `LIMIT_PERCENT` (1.0 in `ngspice_compare.rs`). **Test:** review; a unit test that the default band is still 1 % of span.

21. **Case folding of subcircuit names.** The parser stores lower-case. **Detect:** show original and stored names. **Mitigate:** write the lower-case name in `subckt =`. **Test:** `NE555` in a file binds as `ne555`.

22. **KiCad footprint pad numbering mismatch.** **Detect:** C3 export/import compares pad numbers to `[[pin]].number`. **Mitigate:** fail the export of that part, not the whole project (same spirit as circuit-level missing-footprint reporting). **Test:** a deliberate pad `A1` versus pin `"1"` fails.

## 2. Risks

| Risk | Mitigation |
|---|---|
| Scope becomes a second EDA symbol editor and never ships | C2 primitives stay small; KiCad remains the rich tool; C1 is useful without a symbol editor |
| Licence accidents with vendor models | Encrypted refusal; official CI; URL+checksum for non-redistributable files |
| Schema invented ahead of `of-parts` and E6 | C1 writes only fields that parse today, plus a versioned, agreed extension (`format`, `model_source`) |
| `validate-parts` official rules break personal packs | Explicit modes (pitfall 12) |
| Price and Money type diverge from A1 | Stay on `price_zar_cents` until A1 is accepted (open decision) |
| BOM first-variant rule surprises users | Warning + later placement field |
| Canonicalisation noise in `parts/` | One official formatter and goldens (pitfall 17) |
| Datasheet extraction invents figures | C3 proposals only; citations required |
| Standalone packaging forks the code | Feature gates / a thin entry crate at C5 |
| Multi-unit and hidden pins poorly imported | Refuse or flatten with an explicit warning until a decision |

## 3. Open decisions that need a maintainer call

Please answer these before or during C1 (1–7, 12) and before C3 (8–11). Until then, implementers must not guess a silent default that would be hard to migrate.

1. **Command names.** `ohmforge part new` / `part check` / `part fmt` versus the older `ohmforge new-part` spelling in `DESIGN_AND_PLAN.md` §13.4. Recommendation in this design: the `part …` subcommand family.
2. **Schema version field.** Add `format = 1` on `[part]`, or a top-level `format` like circuit files, or something else? When does it become required for official packs?
3. **Official versus personal validation.** Agree the two modes and the default (this design recommends: official inside the repo `parts/` directory, personal otherwise, always printed in the report).
4. **Price type.** Stay on `price_zar_cents` until parts-availability A1 lands a `Money` type in millionths, or introduce a dated field now? Recommendation: stay on cents, and do not invent `fetched_at` in a way that conflicts with A1.
5. **Variant selection in placements.** When should a circuit file name the variant so BOM v1 stops being “always the first”? This probably belongs with E6.
6. **Multi-unit parts** (dual op-amp, multi-gate). In C1–C3, refuse / flatten, or design units now?
7. **Pack-local tests.** TOML fixtures the standalone tool can run, or Rust tests in `of-components` only for official parts?
8. **Citation TOML shape.** Inline `source = { url, page, rev, date }` versus a shared `sources` table (already open in [`../compatibility-checks.md`](../compatibility-checks.md) §7).
9. **Symbol storage** once E6 lands: SVG, a TOML `[symbol]` of primitives, or SVG generated from TOML in CI?
10. **Pinned KiCad format version**, shared with [`../kicad-export.md`](../kicad-export.md).
11. **Relicensing** a personal pack into the official CC-BY-SA tree: process and required statements.
12. **Instrument profiles** in this tool in P3, or remain hand-written until after C2?
13. **Stale official data:** should a follow-up PR fix `espressif.esp32-c3` `[electrical].idd_modem_sleep` (5 mA typ) to match the 23 mA / 16 mA model, independently of the creator?

Recommendations above are defaults for discussion, not unstated implementation choices.
