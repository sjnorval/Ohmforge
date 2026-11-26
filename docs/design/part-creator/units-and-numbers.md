# Part creator: units and numbers

Status: proposed (design only). Grounded in `of_netlist::spice::parse_value` (`crates/of-netlist/src/spice.rs`) and `ParamSpec` (`crates/of-parts/src/lib.rs`).
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

Two number languages exist in Ohmforge. Mixing them is the most likely way a part will simulate a thousand times too small or too large.

## 1. `part.toml` is raw SI, not SPICE text

Values in `[analog].params` and in `[electrical]` tables are TOML numbers deserialized as `f64`. They are **already in SI units**: ohms, farads, henries, volts, amperes, seconds, kelvin. Examples from the tree:

- `resistance = { typ = 1000.0, tol = 0.01, unit = "ohm" }`
- `roff = { typ = 100e6, unit = "ohm" }` — scientific notation in TOML is fine
- `tc1 = { typ = 100e-6, unit = "1/K" }`
- `bounce = { max = 5e-3, unit = "s" }`

The creator UI may *display* `1 kΩ` or `5 ms`, but it must **write** `1000.0` and `5e-3`. A TOML string `"1k"` will not deserialize as `ModelParam::Value(f64)` and will fail `parse_part`. A bare `1k` is invalid TOML.

Units are strings (`ohm`, `V`, `A`, `s`, …), not parsed by `of-parts`. Prefer the spellings already in the library (`ohm`, not `Ω`, in the file). The UI may render the symbol.

## 2. SPICE decks use `parse_value`

`ohmforge run` on a `.cir` file, subcircuit flattening in `of-components`, and any imported `.lib` go through `of_netlist::spice::parse_value`. That function:

1. Trims and converts the token to **ASCII lowercase**.
2. Reads a decimal mantissa (optional exponent with `e`).
3. Applies a scale suffix from the remainder.

Documented suffixes (from the module rustdoc and the `match` in `parse_value`): `f p n u m k meg g t mil`, then any trailing unit letters (`10uF`, `4.7kohm`). Also `a` (atto, 1e-18), which the rustdoc line does not list but the code accepts.

| Token | What the user often means | What `parse_value` does |
|---|---|---|
| `1k`, `1K` | 1000 | 1000 (lowercased to `1k`) |
| `1meg`, `1MEG` | 1×10^6 | 1×10^6 |
| **`1M`** | often “mega” in casual notes | **1×10^{-3}** (`1M` → `1m` → milli). This is the classic trap. |
| `1m`, `1mohm` | milli | 1×10^{-3} |
| `1u`, `1uF` | micro | 1×10^{-6} |
| `1μF`, `1µF` | micro | **Fails or is not micro**: only ASCII `u` is a suffix. A leading UTF-8 micro sign is not a valid mantissa, so the token does not parse. |
| `1mil` | sometimes “million” | **25.4×10^{-6}** (a thousandth of an inch, by the SPICE `mil` suffix) |
| `1,0` or `1,000` | locale decimal / thousands | **Does not parse** (comma is not a decimal separator; the parser stops at the first non-number byte and then rejects a remainder that is not a known suffix) |
| `10uF` | 10 microfarads | 10×10^{-6} (unit letters after `u` are ignored) |

The creator and the SPICE importer must:

- Show the **parsed SI value** next to every suffix the user types (`1M` → “0.001 (milli). Did you mean 1meg?”).
- Reject a token that does not parse, rather than store 0 or skip it.
- Never write SPICE suffixes into `part.toml`.
- When *exporting* a deck, use `1e6` or `1meg`, never `1M`.
- Accept only `.` as the decimal mark in the UI; if a comma is typed, explain it.

## 3. Tolerances and corners

`ParamSpec` in `of-parts`:

- `tol` is **relative**, `0.01` means 1 %, not 1 Ω.
- If `min`/`max` are absent, `low = typ * (1 - tol)` and `high = typ * (1 + tol)`.
- If `typ` is absent, `typical()` is the mid-point of `min` and `max`, or whichever one is present.
- `Corner::Min` / `Max` / `Deviation(k)` with `k` in `[-1, 1]` feed the analog bridge (`of-components` builds a board at a corner).
- `tolerance()` derives a relative spread from min/max around typ when `tol` is missing. If typ is 0, relative tolerance is `None`.

Pitfalls:

- Writing `tol = 1` meaning “1 %” gives a ±100 % spread.
- Asymmetric datasheet min/max (for example `min = 0.9`, `typ = 1.1`, `max = 1.5`) should be stored as min/typ/max, not as a single `tol`.
- `ParamSpec` has `deny_unknown_fields`: a typo key is a parse error, which is good.

The creator should display both “±1 %” and the implied min/max, and write `tol` only when the datasheet states a symmetric relative tolerance.

## 4. Conditions and temperature

There is no first-class `temp` or `vcc` field on a rating today. Conditions live in `note` (`"VCC = 15 V, IOL = 50 mA"`, `"at 100 V DC"`, `"thick film, +/-100 ppm/K"`). `.temp` is a SPICE directive the parser understands at deck level (`spice.rs`).

Until a structured conditions table exists, official ratings that depend on supply, temperature or load **must** put those conditions in `note`. “Check model” fixtures must use the same conditions (the NE555 tests already distinguish 5 V and 15 V cases in `crates/of-components/tests/ne555_datasheet.rs`).

`tc1`/`tc2` are per-degree-K tempcos on primitives. Do not treat them as ratings.

## 5. Float precision and formatting

- Store TOML numbers that round-trip through `toml` / `f64` without pretty but lossy rounding. Prefer decimal or scientific notation that shows the datasheet digits (`13.3`, `30e-9`), not a 17-digit dump.
- Canonical formatting (C1 `ohmforge part fmt`) must be deterministic so official packs do not churn in git. Pin a style: period decimal, no thousands separators, scientific notation for magnitudes below 1e-3 or above 1e6 unless the file already uses a clear decimal.
- Comparisons in tests should follow the existing patterns: exact for integers and pin maps; relative or absolute bands for analogue values (see [verification.md](verification.md)). Do not compare raw `f64` bit patterns from two formatters.
- `price_zar_cents` is `u64` integer cents. Never store a fractional rand in that field (`12` means R0.12, `4590` means R45.90 on the ESP32-C3 module offer).
