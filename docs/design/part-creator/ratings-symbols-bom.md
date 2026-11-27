# Part creator: ratings, citations, symbols and BOM

Status: proposed (design only). Grounded in `[electrical]` as used in `parts/**/part.toml`, [`../compatibility-checks.md`](../compatibility-checks.md), [`../kicad-export.md`](../kicad-export.md), `crates/of-bom/src/lib.rs`, and [`../parts-availability.md`](../parts-availability.md). Datasheet-fill and KiCad-library sources: [`sources.md`](sources.md) and [`../ecosystem-integrations.md`](../ecosystem-integrations.md).
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).

## 1. Ratings and citations

### 1.1 What exists today

Ratings live in the untyped `[electrical]` map. Citations are free-text `note` strings plus an optional `[part].datasheet` URL. Good examples: `ti.ne555` (document id SLFS022K, section numbers, conditions in the same note). Weak examples: a typ/max with no note, or a note that does not name a document.

[`compatibility-checks.md`](../compatibility-checks.md) already requires: a check that uses a rating must have a source (document, revision or date, and page or clause). Unsourced numbers are not a hard fail. It sketches a structured `source = { url, page, rev, date }` table (illustrative, not in the schema). That shape is still an open question there; the creator must follow the decided schema once it exists, and until then write consistent notes.

### 1.2 Format to use now (official packs)

For each numeric rating the creator writes:

```toml
vcc = { min = 4.5, max = 16.0, unit = "V", note = "SLFS022K §5.3 Recommended Operating Conditions, NE555" }
```

The note should contain, in this order when known: document id or title, revision or date, section or table or figure or page, and operating conditions (supply, temperature, load). The datasheet URL stays on `[part].datasheet`.

C1 official validation: if `verification = "datasheet"`, every rating that a published check will use (supply range, Vf, logic levels, abs-max currents) needs a `note` that is not empty. A rating with a `source` table but no page/clause fails, matching the acceptance line already in the compatibility-checks note.

### 1.3 Abs-max versus recommended, and derating

Store them as **separate keys**, never as one blended min/max. Suggested names, aligned with the compatibility-checks inspector: `vcc` / `vdd` for the recommended operating range; `vin_max`, `vceo`, `if_max` and similar for absolute maxima already used in the tree (`vin_max` on the AMS1117 part, `if_max` on LEDs, `vceo` on the NPN). The inspector must label abs-max more strongly than recommended (that note already says “a stronger finding at abs-max”).

Derating (current versus temperature, voltage versus altitude, and so on) has no schema. If a datasheet curve matters, put the condition in `note` and do not silently scale the rating. Thermal resistance already appears as `theta_jc` with unit `K/W` on the AMS1117 part; keep that spelling if you add it.

### 1.4 How checks consume ratings

P3 first-slice rules (compatibility-checks §4) need cited fields: LED without a series resistor (topology, may need no datasheet), supply outside the cited recommended range, Vih/Vil versus Voh/Vol at that supply, missing decoupling when a part cites it. Until `[interface.*]` exists, those rules should read documented `[electrical]` keys and skip the rule when the field or its citation is missing (`pending`, not a hard fail). The creator should show which planned rules can already fire for this part.

## 2. Symbols and graphics

`[symbol]` and breadboard artwork are **not in the schema yet** (E6). C1 may emit a placeholder `symbol.svg` / `breadboard.svg` in the folder, but must not invent a parallel format that E6 then throws away. Until E6 lands, official packs stay data-only, as they are today.

When E6 and C2 land, the following rules apply so the later KiCad path does not explode.

- **Grid and origin.** Pick a documented grid (recommend 1.27 mm or 2.54 mm, stated in the E6 RFC) and a pin-end origin convention (which corner or centre of the body is (0, 0); pins snap to the grid). Breadboard art is true 2.54 mm pitch ([`../breadboard.md`](../breadboard.md)).
- **Pin length.** Keep one default pin length so automatic routing and hop clearances stay predictable (`wire-routing.md`).
- **Text and fonts.** Store text as text, not outlines. Do not depend on a proprietary font; use a documented open family or the same stroke font the schematic view already uses.
- **Rotation and mirroring.** Store the symbol in its default orientation; instance rotation lives in the circuit file, not in the pack. Import that remaps pins because the symbol was mirrored must show the remapping and wait for confirmation.
- **Pin names versus numbers.** The graphic must carry both, and they must match `[[pin]]`. A mismatch is a C2 validation error.
- **KiCad mapping pitfalls (C3):**
  - Units: KiCad symbol units versus our single-unit parts. A dual op-amp in KiCad is not representable until multi-unit is decided; refuse or flatten with an explicit warning, do not silently drop unit B.
  - Pin electrical types: KiCad types do not map 1:1 onto `power` / `ground` / `input` / `output` / `io` / `passive`. Show the mapping table and let the user override. Hidden KiCad power pins become explicit `[[pin]]` entries (`kind = "power"` or `"ground"`).
  - Footprint pad numbers must match `[[pin]].number`. A SOIC pad `1` that is not our pin `"1"` is a hard fail on export until remapped.
  - Official packs still **reference** footprints by name only (`parts/LICENSE.md`). We may *export* a symbol we authored; we do not vendor a third-party KiCad library into `parts/`.
  - The pinned KiCad format version is shared with [`../kicad-export.md`](../kicad-export.md) (still an open question there).

## 3. BOM and MPN

`crates/of-bom/src/lib.rs` builds BOM v1 from placements: the first `[[variant]]`, then the first offer with `price_zar_cents`, else the first offer. Prices are integer ZAR cents; `Bom::format_zar` prints `R12.34`. There is no currency field (ZAR is implied). There is no lifecycle field. There is no date on the offer.

Creator rules:

- **MPN normalisation.** Store the manufacturer’s canonical spelling (including package suffix). Do not case-fold or strip suffixes in the file. Display a warning when two variants differ only by suffix (`NE555P` vs `NE555DR`) so the author sees that BOM v1 will pick the first. Prefer the datasheet’s order, and put the intended default first until circuit files can name a variant.
- **Packaging suffixes** are different variants, not one MPN with a comment. `ti.ne555` is the example to follow (DIP-8 and SOIC-8).
- **Lifecycle / obsolescence** is not in the schema. A1 / availability work plans lifecycle in a separate `availability/` dataset, not in the CC-BY-SA `part.toml`. The creator may allow a free-text note on the variant, but must not invent a field that then disagrees with A1.
- **ZAR snapshots.** Write `price_zar_cents` only from a source the author can name, and treat it as dated-but-undateable until A1 adds `fetched_at`. The UI must label the figure “snapshot, not live”. Official packs should not pretend the price is current.
- **Millionths.** `parts-availability.md` plans a `Money` type in millionths of a currency unit. That is a future migration. Do not write floating-point rands, and do not write millionths into `price_zar_cents`.
- **Suppliers.** Keys already used include `lcsc`. A2 will add more. Keep the key lower-case. A URL is optional but useful (`https://www.lcsc.com/product-detail/C22805.html`).
- **No variant** is allowed for bench equipment (`bench.psu`). It is not allowed for official circuit parts that will appear on a BOM.

Hand-entered BOM overrides live in the *project* (`bom.toml` in the examples-and-sharing / availability plans), not in the part pack, except for the dated snapshot offers on official variants.
