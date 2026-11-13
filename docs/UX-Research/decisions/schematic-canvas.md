---
title: d1 — Schematic canvas: grid-disciplined, first-class nets
type: decision
domain: ux
status: stable
updated: 2026-11-03
source: https://github.com/sjnorval/Ohmforge/docs/DESIGN_AND_PLAN.md §9, §14 | Synergy Codes (schematic editor hard parts)
confidence: high
tags: [design, schematic-editor, grid, first-class-nets, erc]
---

# d1 — Schematic canvas: grid-disciplined, first-class nets

## Decision

Build the schematic editor as a **grid-disciplined, first-class-nets canvas** in the egui immediate-mode framework: pins keep identity through moves and redraws, nets are model objects (named, power symbols, implicit connectivity), and routing is orthogonal with snap-to-grid.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| Generic node editor (e.g. ngDiagram-style) | ❌ Too late | The diagram canvas is generic; the circuit **semantics** (pins, nets, junctions, ERC) have to be built separately and will dominate |
| Full CAD/ECAD toolchain | ❌ Over-scope | Sum of the rest; not justified at pre-alpha |
| Plain immediate-mode canvas (no rules) | ❌ Credibility risk | Wires that look connected but aren't destroy trust |

## Evidence

- *"The grid is a connectivity requirement, not decoration."* — Synergy Codes (2026), on why snapping belongs in the canvas.
- Stable pin identity, first-class nets, junction topology, orthogonal routing are the non-negotiables that separate a production schematic editor from a canvas demo (Synergy Codes, 2026).

## Trade-offs

- egui immediate mode means the interaction logic (snapping, junction handling, undo/ECO) is custom code — more up-front work, but no heavy third-party dependency.
- Hierarchy (multi-sheet) is deliberately **deferred** to P3; P2 is single-sheet so the canvas can stay shippable.

## Status / next

Implement in EVT: snapping, first-class nets, implicit power nets (VCC/GND), ordered orthogonal routing, juncture cleanup, branch/merge-back, ERC (power rail, output-output, unconnected required inputs). Leave hierarchy + ERC depth for later phases.
