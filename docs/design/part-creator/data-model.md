# Part creator: data model

Status: proposed (design only). Grounded in `crates/of-parts/src/lib.rs` as of `main` through PR #99.
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

This note is the field-by-field contract for what the creator reads and writes. Types below are the Rust types in `of-parts`. Anything marked **planned** is not in `PartFile` today and needs a schema version bump (section 8) before official packs use it.

## 1. File layout

One folder per part, as `parts/README.md` and `CONTRIBUTING.md` already say:

- Official: `parts/<category>/<manufacturer>-<part>/part.toml`, or `parts/<category>/<part>/` for generics (`parts/passives/resistor`).
- A SPICE macromodel lives next to `part.toml` and is named by `[analog].file` (relative path only). Examples: `parts/timers/ne555/ne555.lib`, `parts/regulators/ams1117/ams1117.lib`.
- Instrument profiles are a *different* schema (`parts/instruments/<vendor>.<model>/instrument.toml`) and are out of scope for C1–C3.
- `load_dir` in `of-parts` walks every `part.toml` below a directory, sorted by path. It does not require the folder name to match `part.id`. The creator should still follow the folder convention, and C1 validation for official packs should warn (then, once decided, error) on a mismatch.

There is no `format` or schema-version field today.

## 2. `[part]` — `PartMeta`

| Field | Type | Required today | Notes |
|---|---|---|---|
| `id` | `String` | yes (serde) | Stable library id, for example `ti.ne555`, `generic.resistor`, `espressif.esp32-c3`. Used as the key in `of_bom::load_parts_map` and in circuit files. **Do not rename**; a new id is a new part. The unit test `every_library_part_parses_and_checks` requires uniqueness across `parts/`; `PartFile::check` and `validate-parts` do not. C1 official validation must. |
| `name` | `String` | yes | Display name. |
| `category` | `String` | yes | Path-like, free string. Values in the tree today include `passive/resistor`, `passive/capacitor`, `passive/potentiometer`, `optoelectronic/led`, `discrete/bjt`, `electromechanical/switch`, `power/regulator/linear`, `sensor/environmental`, `ic/timer`, `ic/mcu`, `instrument/power-supply`. The creator should offer this list and allow a new path, not invent a second taxonomy. |
| `datasheet` | `Option<String>` | no | URL only. Never commit the PDF (`parts/LICENSE.md`). Official datasheet-verified parts should have this set. |
| `model` | `String` | yes | Behaviour reference. Values in the tree: `rust:resistor`, `rust:capacitor`, `rust:led`, `rust:bjt`, `rust:switch`, `rust:potentiometer`, `rust:bme280`, `rust:esp32c3`, `rust:bench-psu`, `spice:ne555.lib`, `spice:ams1117.lib`. The comment on `bosch.bme280` also documents `wasm:bme280.wasm`. Keep the `rust:` / `wasm:` / `spice:` prefixes. For a `subckt` part, `model` should stay consistent with `[analog].file` (today `ti.ne555` has `model = "spice:ne555.lib"` and `file = "ne555.lib"`). C1 must flag a mismatch. |
| `verification` | `String` | yes | Documented as `datasheet`, `bench-measured` or `community` (`PartMeta` rustdoc and the BME280 comment). **Not enum-checked** by `check()` or `validate-parts`. C1 must allow only those three strings. |

## 3. `[[pin]]` — `Pin`

Serde rename: the TOML table is `[[pin]]`, the field is `pins: Vec<Pin>`.

| Field | Type | Required today | Notes |
|---|---|---|---|
| `number` | `String` | yes | Package pin number, not necessarily an integer (`"1"`, `"8"`). Unique per part: `PartFile::check` errors if a number is used twice. Lookup (`PartFile::pin`) tries an exact match on `number` first, then a case-insensitive match on `name`. **Pitfall:** a name of `"1"` and a number `"1"` on different pins would collapse on lookup. The creator must forbid a name that equals another pin's number. |
| `name` | `String` | yes | Functional name (`VCC`, `GND`, `TRIG`, `GPIO0`). |
| `kind` | `Option<String>` | no | Rustdoc lists `power`, `input`, `output`, `io` or `passive`. The tree also uses **`ground`** (`espressif.esp32-c3` pin 2). `check()` does not validate the vocabulary. The creator treats the vocabulary as `power`, `ground`, `input`, `output`, `io`, `passive` (six kinds) and rejects anything else. |

Rules already enforced by the engine plan and by current parts:

- List every pin that can be wired, including supply and ground. There are **no hidden pins** and no hidden connection to ground. A KiCad symbol that hides a power pin must become an explicit `[[pin]]` on import (see the ratings/symbols note).
- **Multi-unit** devices (dual op-amp, two gates in one package) are not represented. Do not invent unit-A / unit-B in C1. This is an open decision.
- `[analog].pins` is a list of numbers *or* names, in **device terminal order** (rustdoc on `AnalogModel`). For `subckt` this is the `.subckt` port order, not the visual order around the symbol, and not the package pin-number sequence if those differ.

`PartFile::check` pin rules today: unique numbers; every `[analog].pins` entry must resolve through `pin()`; kind-specific counts (`npn`/`pnp`/`potentiometer` need 3, `subckt` any length, every other analog kind needs 2). It does **not** require unique names, a `kind`, or a minimum number of pins on parts without `[analog]` (the ESP32-C3 has many pins and no `[analog]` section, which is correct).

## 4. `[electrical]` — untyped today

```rust
pub electrical: BTreeMap<String, toml::Value>
```

This is **not** a `Param` map. The `Param` struct (`min`/`typ`/`max`/`unit`/`note`) exists in the same file but is not used to deserialize `[electrical]`. Keys are free-form. Values in the tree are usually tables `{ min, typ, max, unit, note }`, but also:

- arrays: `bosch.bme280` has `i2c_addresses = [0x76, 0x77]`;
- conditions stuffed into `note` (for example NE555 `vol = { typ = 0.4, max = 0.75, unit = "V", note = "VCC = 15 V, IOL = 50 mA …" }`).

Units actually used: `V`, `A`, `W`, `ohm`, `s`, `deg`, `dB`, `K/W`. There is no check that `min ≤ typ ≤ max`, that a unit is present, or that a note exists.

**C1 policy:** keep the free-form map so existing parts still parse, but when the creator *writes* a numeric rating it must use a table with SI values (see [units-and-numbers.md](units-and-numbers.md)), a `unit`, and a citation for official packs (see [ratings-symbols-bom.md](ratings-symbols-bom.md)). Do not bind `[electrical]` automatically to the model; they can drift. A real example on `main`: `parts/mcu/esp32-c3/part.toml` still has `idd_modem_sleep = { typ = 5e-3, …, note = "quiescent load used by Esp32c3Component" }`, while the component and the demo files use the datasheet 23 mA / 16 mA figures. “Check model” must compare cited currents to what the model actually draws, or at least warn.

Planned (compatibility checks / D2): a structured interface/limits block with `source = { url, page, rev, date }`. Until that schema is accepted, official citations stay in `note` plus the datasheet URL, and the creator should prefer a consistent note style (document, section or page, conditions).

## 5. `[analog]` — `AnalogModel` (`deny_unknown_fields`)

| Field | Type | Required | Notes |
|---|---|---|---|
| `kind` | `String` | yes | Must be one of `ANALOG_KINDS` in `of-parts`: `resistor`, `capacitor`, `inductor`, `diode`, `led`, `npn`, `pnp`, `load`, `switch`, `potentiometer`, `subckt`. There is **no MOSFET kind** in the parts bridge (`COMPONENTS.md` already says so); MOSFETs can appear in a SPICE deck but not in a `part.toml` until a kind or a subcircuit is added. |
| `pins` | `Vec<String>` | yes | Terminal order, as above. |
| `params` | `BTreeMap<String, ModelParam>` | default empty | See below. |
| `file` | `Option<String>` | required if `kind = "subckt"` | Relative to the part directory. |
| `subckt` | `Option<String>` | required if `kind = "subckt"` | Name inside `file`. |
| `note` | `Option<String>` | no | Origin and limits. Used well on `ti.ne555`. |

`ModelParam` is untagged: a bare `f64` or a `ParamSpec` table (`min`, `typ`, `max`, `tol`, `unit`, `note`) with `deny_unknown_fields`. `check()` requires every param to have a typical value (`typ`, or the mid-point of `min`/`max`, or just `min` or `max`).

Primitive param names expected by `of-components/src/analog.rs` (from `AnalogModel` rustdoc and the bridge):

| kind | pins (order) | params |
|---|---|---|
| `resistor` | 2 | `resistance`; optional `tc1`, `tc2` |
| `capacitor` | 2 | `capacitance`; optional `esr`, `esl`, `tc1`, `tc2` |
| `inductor` | 2 | `inductance`; optional `rser` |
| `diode` / `led` | 2, anode then cathode | SPICE diode names (`is`, `n`, `rs`, …); LED extras `vf`, `if`, `wavelength`, `eqe` |
| `npn` / `pnp` | 3, C B E | Gummel-Poon names (`is`, `bf`, …) |
| `load` | 2 | `voltage` and/or `current` |
| `switch` | 2 | `ron`, `roff`; placement param `closed` (and engine-path `bounce`) |
| `potentiometer` | 3, end–wiper–end | `resistance`; placement param `position` (0 to 1, default 0.5) |
| `subckt` | port count of the subcircuit | passed through as subcircuit parameters |

Placement-time parameters (`resistance` on a generic resistor, `closed`, `position`) are not variant fields; they belong on the circuit placement. The creator should document them on the part (as the resistor `part.toml` comment does) but not bake a 330 Ω typ into a generic resistor's only variant by accident: today's `generic.resistor` has `params.resistance.typ = 1000.0` in `[analog]` and a 330 Ω *variant* used on the demo boards. That split is easy to get wrong.

## 6. `[[variant]]` — `Variant`

| Field | Type | Required | Notes |
|---|---|---|---|
| `label` | `String` | yes | Human label, including package where it matters (`DIP-8`, `SOIC-8`, `0603, 330 ohm, 1 %`). |
| `mpn` | `String` | yes | Manufacturer part number. See [ratings-symbols-bom.md](ratings-symbols-bom.md) for normalisation. |
| `manufacturer` | `String` | yes | |
| `footprint` | `Option<String>` | no | **Name only**, for example `Package_DIP:DIP-8_W7.62mm`, `Resistor_SMD:R_0603_1608Metric`. `parts/LICENSE.md` forbids committing third-party footprint libraries. |
| `offers` | `Vec<Offer>`, default empty | no | See below. |

`Offer`: `supplier` (string key, e.g. `lcsc`), `sku`, optional `url`, optional `price_zar_cents` (`Option<u64>` — **integer ZAR cents**, not rand, not millionths). There is **no date, currency, or quantity-break field** on `Offer` today. Staleness cannot be represented in the current schema; A1 in `parts-availability.md` plans `fetched_at` and a `Money` type in millionths. The creator must write `price_zar_cents` until that migration is accepted (open decision).

**BOM pitfall:** `Bom::from_placements` (`crates/of-bom/src/lib.rs`) uses the **first** `[[variant]]` only, then the first offer that has a price, else the first offer. `ti.ne555` lists DIP-8 (`NE555P`) first and SOIC-8 (`NE555DR`) second, so a BOM today always buys the DIP. The creator must show this clearly, and E6 / circuit files need a way to name the chosen variant (open decision).

Bench instruments (`bench.psu`) have no variant and are omitted from the BOM on purpose. Official *circuit* parts need at least one orderable variant; `validate-parts` does not yet enforce that (BOM build fails with `NoVariant` instead).

## 7. `[license]` — `PartLicense` (required)

| Field | Type | Required | Notes |
|---|---|---|---|
| `data` | `String` | yes | Official tree: `CC-BY-SA-4.0`. |
| `artwork` | `String` | yes | Official tree: `CC-BY-SA-4.0`. |

`validate-parts` and the `of-parts` library test require both fields to be exactly `CC-BY-SA-4.0`. That is correct for `parts/` and **wrong** for a personal pack with another declared licence. C1 needs an official mode (default when the path is the in-tree library) and a personal mode (licence must be *declared*, not necessarily CC-BY-SA). See [licensing.md](licensing.md).

**Planned:** a model-source record (`[model_source]` or `[license].model`) with URL, checksum, retrieved date, licence, and a redistributable flag. Not in the schema today.

## 8. Versioning, stable IDs, migration

Today a `part.toml` has no version. Planned, as part of C1 or a tiny RFC before C1 lands:

- Add `format = 1` on `[part]` (or a top-level `format`, matching circuit files' `format = 1` in RFC 0003). **Name is an open decision.**
- `format = 1` is today's schema plus optional planned fields (`model_source`, structured citations, `[symbol]`) that must be ignored by old loaders if we use `#[serde(default)]` and do **not** put `deny_unknown_fields` on `PartFile` (it currently does not; `AnalogModel` and `ParamSpec` do). Adding a field to `AnalogModel` without a default will break every existing subcircuit part.
- A future `format = 2` may move prices to millionths, tighten `[electrical]`, and add `[symbol]`. Old files migrate with `ohmforge part fmt` / `validate-parts --write`. Never rewrite official packs in a way that changes ids, pin numbers, or subcircuit port order.
- `part.id` is stable forever. Aliases, if ever needed, are a new optional field, not a silent rename.

## 9. What the creator writes in C1 versus later

**C1 writes only fields that already parse**, plus (if the RFC/schema bump is accepted in the same slice) `format` and `[model_source]`. It does not invent `[symbol]` ahead of E6.

**C2** edits the same files, and a first symbol graphic once E6 pins the field.

**C3+** add KiCad mapping tables next to the part (CC-BY-SA data we author, not vendored libraries) and structured citations once compatibility-checks settles the TOML shape.
