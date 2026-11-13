---
title: d5 — UI shell: soft-front-panel instrument shell
type: decision
domain: ux
status: stable
updated: 2026-11-03
source: NI ELVIS III | Digilent WaveForms (Analog Discovery 3) | ux-research.md §2
confidence: high
tags: [design, shell, instrument-shell, soft-front-panel]
---

# d5 — UI shell: soft-front-panel instrument shell

## Decision

One window with a big central canvas for the active instrument and a **collapsible right-hand strip** of instrument controls + probes; a left toolbar for the project. No permanent full block front panel.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| Fixed block front panel (all knobs/BC-switch style) | ❌ Wastes vertical space | Reads as a mock-up; can't grow |
| Every instrument as a full-screen window | ❌ Orange-ish fragmentation | Editors open the scope and the logic-analyser at once |

## Evidence

- Modern instruments moved to *soft front panels*: NI ELVIS III *"seven instruments through a minimal installation"*; Analog Discovery 3 *"friendly user interface that has the feel of traditional benchtop devices"* (Digilent, n.d.).
- Takeaway: preserve the hardware feel (the audience knows scopes), but keep controls contextual and collapsible.

## Trade-offs

- "Hardware feel" must survive in a collapsible strip; some controls live one click away.
- The active-instrument canvas convention means panels must be clearly re-identifiable when you switch instruments.

## Status / next

Implement EVT: top-level window shell; collapsible right panel (per instrument); left project toolbar; instrument tab or pane activation. Keep the same visual language across scope, logic-analyser, multimeter, serial.
