# KiCad and fabrication export

Status: proposed (design only, no code). The roadmap already has P3 “KiCad schematic/netlist export (`of-kicad`) and XLSX BOM export” and an item to explore Gerber, drill, BOM and pick-and-place via that export.
Date: 2026-11-25.
Related: [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §9 and §11; [`wire-routing.md`](wire-routing.md); [`engine-components.md`](engine-components.md) E6; [`parts-availability.md`](parts-availability.md); [`interconnects.md`](interconnects.md) (copper weight in the PCB export).

Ohmforge is not a PCB layout tool. Once a circuit is a real, orderable design, the user should be able to take it into KiCad as a schematic and netlist, with footprints and MPNs preserved, and from there to a fabricator. This note describes that export path and the limits of the “explore fabrication” item.

## 1. Current state

BOM v1 can write a CSV and a build checklist for the bundled demo boards. There is no `of-kicad` behaviour yet, and no Gerber path. Symbol and footprint fields in `part.toml` are only as far as the parts library has taken them.

## 2. Goals and non-goals

Goals:

- Export a KiCad schematic and netlist from an E4 circuit file, including symbols, reference designators, values, MPNs, and footprints when `part.toml` has them.
- Export an XLSX BOM (and keep the CSV) that a human can send to a supplier, with the same rows as the in-app BOM.
- Document a fabrication path: the user opens the export in KiCad, lays out the board there, and produces Gerber, drill, BOM and pick-and-place (CPL) with KiCad. Ohmforge may later write a starter `.kicad_pcb` with footprints unrouted if that stays small and lawful; it is not required for 0.2.
- Keep licensing clean: we generate KiCad’s documented file formats; we do not copy KiCad source, and we do not link GPL code into the Ohmforge workspace.

Non-goals:

- A full PCB editor, autorouter, or 3D enclosure tool inside Ohmforge.
- Promising that every official example is fabricable without a layout pass.
- Supporting every historical KiCad format. Pin a current, documented format and record it in the exporter.

## 3. User-facing behaviour

File → Export → KiCad writes a folder with the schematic, the netlist, and the BOM (CSV and XLSX). The official blink or dimmer example, after E6, exports and opens in the pinned KiCad version with symbols and refs intact. A short guide (P3 illustrated guide, then the P6 user guide) shows “Export, open in KiCad, assign any missing footprints, lay out, plot Gerbers”. If a part has no footprint yet, the export lists it as missing rather than inventing one.

## 4. Technical approach

- `of-kicad` is a Rust crate in the workspace (AGPL-3.0-or-later) that emits text files from the circuit and from `part.toml`. Mapping tables (symbol name, pin numbers, footprint library id) live with the part data (CC-BY-SA-4.0).
- Schematic wire geometry may use the same routed paths as the schematic view, as the roadmap already says. If a path cannot be represented, fall back to a clean but unrouted schematic rather than a corrupt file.
- Tests: golden exports for official examples, opened or parsed in CI (a syntax check is the minimum; opening in KiCad in CI is better if a licence-compatible, reproducible install can be pinned).
- Fabrication outputs, if generated at all from Ohmforge, go through this path or through a clearly labelled experimental exporter; default docs tell people to plot from KiCad.

## 5. Phasing

- **P3 / 0.2:** schematic + netlist + XLSX/CSV BOM for official examples; documented KiCad version; missing-footprint reporting. Needs E4, E6, and the schematic router if exported wires follow those paths.
- **P3 (explore) / later:** a written decision on starter `.kicad_pcb` and on whether Ohmforge ever emits Gerber itself. Copper weight and clearance (W2, W3) belong in that later slice, as already planned.

## 6. Acceptance criteria

- The official blink (or dimmer) circuit exports and the schematic opens in the pinned KiCad version with the expected number of parts and nets.
- A part without a footprint is listed, and does not fail the whole export silently.
- CI checks the exporter against goldens.
- No KiCad source is vendored, and `deny.toml` stays free of GPL/LGPL dependencies.

## 7. Open questions

- The pinned KiCad format version, and whether to emit a `fp-lib-table` / symbol library or only reference standard libraries plus a small Ohmforge library we maintain.
- Whether a starter unrouted PCB is in 0.2 or is explicitly deferred.
