# Part creator: sources, datasheet fill and KiCad import

Status: proposed (design only). Workflows for the external sources the part creator may use. The tools themselves, their phases, licence rules and acceptance tests are specified in [`../ecosystem-integrations.md`](../ecosystem-integrations.md).
Date: 2026-11-26.
Parent: [`../part-creator.md`](../part-creator.md).
Related: [`ratings-symbols-bom.md`](ratings-symbols-bom.md); [`licensing.md`](licensing.md); [`ux-and-testing.md`](ux-and-testing.md); [`../kicad-export.md`](../kicad-export.md); [`../parts-availability.md`](../parts-availability.md); [`../../COMPONENTS.md`](../../COMPONENTS.md).

This note is the author-facing map. It does not add schema fields. Anything written into a pack still follows [`data-model.md`](data-model.md), the citation style in [`ratings-symbols-bom.md`](ratings-symbols-bom.md), and the licence rules in [`licensing.md`](licensing.md).

## 1. What may never go into an official pack

- Content from SnapEDA, Component Search Engine, or any similar service whose terms forbid redistribution.
- Vendored official or third-party KiCad symbol and footprint *files* (names only; see `parts/LICENSE.md`).
- Datasheet PDFs, live prices, live stock, inventory exports, or API keys.
- Marking-database dumps of unclear licence.
- Encrypted or non-redistributable vendor models (see [`spice-import.md`](spice-import.md)).

Personal packs may hold user-local files the author already has the right to use; official validation must still refuse the list above.

## 2. Choosing which part to add (P3)

Before spending time on a model, check the official growth list in [`COMPONENTS.md`](../../COMPONENTS.md) and the JLCSearch-guided backlog described in the ecosystem note §2.6. Prefer a part that has a clear manufacturer part number, a datasheet, and (where it makes sense) an LCSC or other documented SKU. Re-check stock at authoring time: a backlog date is not a promise of availability. A teaching or regional part may go ahead with a short recorded reason.

## 3. Datasheet-assisted fill (documented in P3, in-app in P4 C3)

The existing D3 plan (MIT-licensed PDF crates in process; poppler out of process) remains the default for text. Two user-run helpers sit beside it:

1. **Tabula** — extract a ratings table from a *local* PDF the author already has, to CSV. The creator proposes `[electrical]` min/typ/max and a citation `note` (document id, page or table, conditions). The author accepts field by field.
2. **WebPlotDigitizer** — digitise a figure to a point file. The creator stores reviewed points only as a citation-backed typ/max or as a verification fixture, never as an automatic model fit. Ohmforge does not link the AGPL frontend and does not upload images to a third-party cloud.

Both are proposal-only. Units are still raw SI in `part.toml` ([`units-and-numbers.md`](units-and-numbers.md)). Official packs still fail if a check-used rating has an empty `note`.

## 4. KiCad import and footprint names (P3 pin, P4 C3)

The authority for names is a **pinned official KiCad symbol and footprint library** shared with `of-kicad` ([`../kicad-export.md`](../kicad-export.md)). C3 imports a symbol from that library on the author's machine (and a footprint later, when the schema is ready). Missing footprints are listed, not invented.

`kitspace/kicad_footprints` may be a **user-local, secondary** lookup when an official name is hard to find. Do not vendor that collection. Inkscape Electric Symbols are **reference only** for drawing conventions while we author our own IEC and ANSI artwork (C2); do not import the set into `parts/`.

## 5. Variant and marking lookup (P4)

An SMD marking database, if a licence-clean or user-supplied source is chosen (ecosystem note, open decision), may *propose* a candidate MPN. The author confirms it against the datasheet before the variant is official.

## 6. Prices, stock and inventory are not pack fields

Live offers, lifecycle and “I already own this” checks belong to [`../parts-availability.md`](../parts-availability.md) and the ecosystem note (§2.8, §2.9), not to `part.toml` snapshots as if they were current. The creator may write a dated BOM v1 `price_zar_cents` on a variant, labelled as a snapshot, following [`ratings-symbols-bom.md`](ratings-symbols-bom.md).
