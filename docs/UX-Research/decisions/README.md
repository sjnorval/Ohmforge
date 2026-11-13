---
title: Decision Records
type: log
domain: design
status: stable
updated: 2026-11-03
---

# Decision Log

> Why the UX Research workstation and design brief made the calls they made. Each record states the decision, the alternatives considered, the evidence, and the trade-off.
>
> **Repository:** <https://github.com/sjnorval/Ohmforge> · plan anchor: `docs/DESIGN_AND_PLAN.md` §9 (UI and instruments)

---

## Record format

| Field | Purpose |
|---|---|
| `decision` | One-line statement of what we chose |
| `alternatives` | What we ruled out (and why) |
| `evidence` | Source(s) for the choice |
| `trade-off` | What we knowingly accept |
| `link` | Companion: `design-brief.md` screen, `ux-research.md` research |

---

## Decisions captured

| # | Decision | Status |
|---|---|---|
| `d1` | Schematic canvas: grid-disciplined, first-class nets | Stable |
| `d2` | Oscilloscope rendering: GPU per-channel (wgpu) | Stable |
| `d3` | Multimeter: click-to-probe, one tool | Stable |
| `d4` | Logic analyser: protocol decode panels | Stable |
| `d5` | UI shell: soft-front-panel instrument shell | Stable |
| `d6` | UI toolkit: egui/eframe 0.37 (keep, not iced/Slint) | Stable |

> Add a new record here when a design call is made. The companion `design-brief.md` screen notes carry the earlier, screen-level detail.
