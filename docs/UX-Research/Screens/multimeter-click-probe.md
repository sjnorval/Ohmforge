---
title: Multimeter — Click-to-Probe Surface Spec
type: screen-spec
domain: ux
status: stable
updated: 2026-11-13
source: d3 (multimeter-probe.md) + design-brief.md (multimeter) + ux-research.md §5
confidence: medium
tags: [screen-spec, multimeter, click-to-probe, probe, instrument, dmm]
---

# Multimeter — Click-to-Probe Surface Spec

> One tool, many modes — by interaction. The probe is the instrument; the user's highest-frequency action in the simulator.

---

## 1. Surface overview

- One collapsible right-hand panel; when the multimeter is the active instrument, a **probe area** takes the centre of the window (where nets and parts are probed).
- The probe binds to a net or part and turns the DMM into that measurement.
- Header shows the numeric readout + bar scale, with mode and auto/manual range controls.

---

## 2. Probe interaction model

The defining interaction is click-to-probe:

| Action | Result |
|---|---|
| Click a part (e.g. resistor) | → per-part V / I / power / junction-temp |
| Click a net | → net voltage (numeric + bar scale) |
| Click-drag across a net / part | → transient oscilloscope on that net |
| Shift-drag-multi | → multi-point capture (optional) |

---

## 3. One tool, many modes

| Mode | How activated | Output |
|---|---|---|
| Probe | Click a part | Per-part V / I / power / junction-temp |
| Net voltage | Click a net | Net voltage (numeric + bar scale) |
| Transient scope | Click-drag across a net | Transient waveform on that net |
| Auto-range | Trigger on probe | Best range automatically |
| Manual-range | Stepper / rotate | Explicit range |

---

## 4. Numeric + bar scale

- **Numeric readout**: most-used measurement value; auto-scaled.
- **Bar scale**: visual; fills proportionally to the measured value; gives immediate magnitude without reading a number.

---

## 5. Auto-range

- **Auto-range**: trigger on probe; picks the best range automatically.
- **Manual override**: stepper / rotate to set range explicitly.

---

## 6. Validation plan (before EVT)

| Check | Method |
|---|---|
| Probe accuracy | Reference measurement against a known source |
| Auto-range correctness | Sweep values; verify range selection |
| One-tool-many-modes (UI consistency) | Consistent probe→binding semantics across screens |
| Cross-instrument consistency | Probe on schematic; matches instrument panels |
| Touch support | Probe drag on canvas; needs touch-device discovery (UDK target?) |

---

## 7. Design decisions summary

- Probe is the instrument — the highest-frequency action in the simulator.
- One tool re-purposes by interaction (per-part, net, transient).
- Numeric-led + bar scale.
- Auto-range on; manual override available.
