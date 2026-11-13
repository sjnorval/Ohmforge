---
title: d3 — Multimeter: click-to-probe, one tool
type: decision
domain: ux
status: stable
updated: 2026-11-03
source: Keysight (n.d.) — "What is an Oscilloscope Display?" (probe is the instrument) | ux-research.md §3
confidence: high
tags: [design, multimeter, click-to-probe, probe, interaction]
---

# d3 — Multimeter: click-to-probe, one tool

## Decision

Build a single **Multimeter** that re-purposes by interaction: click a part → per-part V/I/power/junction-temp; click a net → numeric readout + floating bar scale; click-drag across a net → transient oscilloscope. Auto-range with manual override.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| Four separate instrument windows | ❌ High friction | Four top-left clicks to measure one net; kills the debug loop |
| DMM + oscilloscope as always-open | ❌ Divergence | Editors reach for both at once; splitting them forces an extra click |

## Evidence

- On a real bench, the probe **is** the instrument (Keysight, n.d.): select a point, read the value directly. The same mental model applies in a simulator, where the "probe" is a click/drag on the canvas.
- The simulator already has scope + logic-analyser + serial; collapsing them into one high-frequency interaction matches how editors think.

## Trade-offs

- One tool serving four roles reads at first, but the interaction is consistent and cheap once learned.
- Requires per-part identity (click → part → V/I/power/junction-temp) plus net identity (click → net → numeric + bar scale), which the canvas already carries.

## Status / next

Implement EVT: per-part readout; per-net numeric + bar scale; click-drag transient capture; range stepper (auto + manual). Keep the transient path on the same surface as the scope so probes can switch readers in a single click.
