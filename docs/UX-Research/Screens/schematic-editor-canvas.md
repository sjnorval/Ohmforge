---
title: Schematic Canvas — Interaction Model Spec
type: screen-spec
domain: ux
status: stable
updated: 2026-11-03
source: d1 (schematic-canvas.md) + design-brief.md (schematic editor) + ux-research.md §5
confidence: medium
tags: [screen-spec, schematic-editor, canvas, interaction, snap-to-grid]
---

# Schematic Canvas — Interaction Model Spec

> Interaction model for the P2 schematic editor canvas. Grounded in `d1` and the research (Synergy Codes, 2026). Visual language is the designer's next step; this is the **behaviour** spec so the canvas is shippable and electrically correct from day one.

---

## 1. Canvas gestures

| Gesture | Action |
|---|---|
| Pan | Middle-drag, or Space + drag |
| Zoom | Scroll wheel; trackpad pinch; or double-tap |
| Select | Pointer-up on a pin, symbol, wire, or net label |
| Multi-select | Pointer-down on empty canvas, drag a selection box |
| Move selected | Drag a selection or symbol |
| Hover cursor | Crosshair (annotated, editable) |

**Grid discipline**: everything snaps to the grid (default ~2048 EMU / configurable). The grid is the *connectivity* requirement, not a cosmetic. Off-grid endpoints would produce "looks connected but isn't" failures.

---

## 2. Component placement

| Step | Action |
|---|---|
| 1. Toolbar | Component catalog (resistor, capacitor, diode, LED, BJT, MOSFET, op-amp, linear regulator, connector, sensor, passive) |
| 2. Drag from catalog | Drops a symbol centered on canvas at grid points |
| 3. Place | Click to confirm position; drag to set orientation |
| 4. Pin annotation | Reference designator auto-assigned (R1, C1, …); editable |
| 5. Edit | Double-click to open a small properties panel (value, footprint, datasheet link) |

- The catalogue reads `part.toml` data (MPN + supplier + price); the placed symbol carries the same identity for the BOM.
- Symbols orient to the majority of their pins (logical up), which differs from pure rotation of the glyph.

---

## 3. Net creation

| Action | Effect |
|---|---|
| Wire pins | Click a pin, drag to another pin → net with automatic junction |
| Junction | Click two points on a wire → branch point (auto-cleanup) |
| Merge | Remove a branch point while the two wires converge → junction cleanup |
| Net label | Type a label (e.g. `VCC`, `GND`) → implicit connectivity to all pins/labels of that name |
| Bus | `D[0..7]` as a structured object, not eight labels |

- Wires are orthogonal by convention and snap to the grid.
- Symbols stay on the grid; wire endpoints are tied to their pins (not free-hand) — moving a symbol repositions wires without changing topology.

---

## 4. ERC (electrical rules checking)

Small rule set surfaced as a **warning list**, not a red screen (see d1):

| Rule | Detection |
|---|---|
| Power rail | Output-to-output conflict, or power pin not connected |
| Output-output | Two outputs driving the same net |
| Unconnected required | Input marked required with no connection |
| Unpowered power pin | Power pin with no source/rail |

Each violation is a low-severity warning in an edge-panel "ERC" view, addressable in one click and cleared by fix.

---

## 5. Interaction & mutation model

| Concern | Approach |
|---|---|
| Undo/redo | Topological, reversible graph mutations (annotion-aware); one level of redo |
| Delete | Branch + junction removal with merge-back semantics; no silent topology loss |
| Duplicate | Shift-drag or Ctrl/Cmd+D copies selection with re-annotated references |
| Move | Symbol moves; pins/wires follow; nets preserve topology |
| Zoom/pan | Mouse wheel + middle-drag; in-canvas zoom box for multi-region |

---

## 6. Probe & instrument bridging

The canvas is the hub for the instruments (d3):

| Action | Instrument switch |
|---|---|
| Click a part | → Multimeter: V / I / power / junction-temp |
| Click a net | → Multimeter: numeric + bar scale; or transient oscilloscope on drag |
| Select a net | → Logic analyser: show this net's edges in the timing view |
| Left-canal bus | → Logic analyser: decode I²C/SPI/UART |

This is the "one tool, many modes" model — the highest-frequency interaction in the simulator.

---

## 7. Open questions for the designer

1. **ERC depth** — P2 rule set only (+info); P3: full layoutlint? Budget the rule set now.
2. **Grid size** — fixed (2048 EMU?) or zoom-sensitive? A zoom-sensitive grid is smoother at all magnifications but the snap-boundary logic is messier.
3. **Touch** — probe drag on the canvas; needs touch-device discovery (UDK target?).
4. **Safety** | operate on immutable snapshots for undo/ECO so no topology is ever lost silently.

---

## 8. Design decisions summary

- Snap-to-grid as a **connectivity** requirement, not decoration.
- First-class nets + implicit power connectivity (`VCC`/`GND` labels) — no explicit wires for rails.
- ERC as a warning list; stable pin identity; reversible graph mutations.
- One tool that re-purposes by interaction (the probe-to-instrument bridge).
