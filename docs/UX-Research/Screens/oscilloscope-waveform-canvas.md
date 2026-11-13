---
title: Oscilloscope — GPU Waveform Renderer Surface Spec
type: screen-spec
domain: ux
status: stable
updated: 2026-11-03
source: d2 (gpu-waveform.md) + design-brief.md (oscilloscope) + ux-research.md §3
confidence: medium
tags: [screen-spec, oscilloscope, gpu, wgpu, waveform-renderer, surface]
---

# Oscilloscope — GPU Waveform Renderer Surface Spec

> Buildable spec for the live oscilloscope waveform surface, for the P2 MVP. Covers the rendering pipeline, data flow, per-channel primitives, controls, and measurement outputs. Grounded in `d2`, `design-brief.md`, and `ux-research.md §3`.

---

## 1. Surface overview

- One collapsible right-hand panel; when the oscilloscope is the active instrument, a **full-width waveform canvas** takes the center of the window.
- GPU-rendered line primitives: **one wgpu texture per channel**, rendered at 60 fps; the CPU egui canvas is not the waveform surface.
- Split surface: time-domain waveform (primary) + a toggleable **FFT / spectrum panel** (realfft) and a **measurement badges row**.

---

## 2. Rendering pipeline

```
CMS core (subject to a hard clock epoch) ──► Data pipeline (demultiplex samples per channel)
      │
      ▼
Render thread (wgpu) ──► Per-channel line-primitive into a texture (60 fps)
      │
      ▼
egui canvas ──► Present the texture + overlays (grid, cursors, rulers, FFT panel)
```

### Data flow

1. The subject is run in quanta; every sample is stamped to a **time-ordered buffer**.
2. A single source of truth: one repository feeding every channel, so every instrument reads the same epoch.
3. The oscilloscope reads a **fixed-width ring buffer** of the latest N samples (configurable, e.g. 8 000 points per channel).

### Render loop (60 fps)

```
frame N ──► copy B buffer slice (time, channel) into uniform buffer
          ──► draw N line primitives (one per channel) to texture
          ──► egui composite texture + grid (background) + overlay (cursors, FFT, nav)
```

- **Object-shape stability**: samples are written into a flat, fixed-layout struct so V8 does not deopt on inconsistent property ordering (see d2).
- **Normalisation**: copy the numeric slice into one flat array before the GPU draw command; no per-frame deep cloning.

---

## 3. Channel model

| Field | Type | Notes |
|---|---|---|
| `id` | `ChannelId` | Stable identifier per instrument |
| `name` | `String` | e.g. `Ch1 (VDD)` |
| `color` | `Color` | UI colour; the trace keeps this |
| `scale` | `f64` | V/div |
| `offset` | `f64` | Volt offset (scroll) |
| `source` | `ChannelSource` | Analog pin, digital net, bus, PSU rail |

- One texture per active channel; 4–8 channels max for the MVP.
- Textures are cleared/compacted on a hard clock epoch to avoid drift.

---

## 4. Controls

### Trigger (the defining control)

| Mode | Behaviour |
|---|---|
| **Auto** | Fill the screen; repeat until a real edge. |
| **Norm** | Hold last image until a real trigger. |
| **Edge** | Trigger on rising/falling edge (setup/hold). |
| **Pulse** | Trigger on a pulse width / level window. |
| **Software** | Button "capture now" → freeze the latest image. |

- Auto mode is the default for the MVP: it always shows a picture while the subject runs.

### Scale

- V/div (vertical zoom) — auto-scale per channel on connect; manual override with a stepper/rotate.
- Units: mV/div, V/div, A/div (multi-meter context).

### Timebase

- Sec/div with a stepper; supports `ns`–`ms` range.
- Trigger delay knob (horizontal offset).

---

## 5. Measurement outputs (auto badges)

Read from the data buffer (not the GPU), so they're cheap and consistent:

| Badge | Formula |
|---|---|
| `Vp-p` | Peak-to-peak voltage |
| `Freq` | Frequency (fundamental) |
| `Rise/fall` | Rise/fall time at 10–90% |
| `Duty` | High-time / period |
| `Crest` | Peak / RMS |
| `Junction temp` | From the thermal model |

Badges live near the waveform; a measurement panel (expanded) shows exact values + cursors.

---

## 6. FFT panel (toggle)

- RealFFT transform of Ch1 (or a selected channel).
- Frequency spectrum with a cursor for the dominant peak.
- Converters: `realfft` (Rust) → displayed as a chart in a side panel.

---

## 7. Interaction

| Gesture | Action |
|---|---|
| Pan time | Horizontal drag / scroll |
| Zoom | Vertical pinch/scroll; in-canvas zoom box |
| Click net/part | → Multimeter probe (d3) |
| Drag across a net | → Transient oscilloscope on that net |
| Cursor drag | Place measurement cursors |
| Auto | Re-capture until a real trigger |

---

## 8. Validation plan (before EVT)

| Check | Method |
|---|---|
| 60 fps | `telemetry` on GPU draw, target ≤ 10 ms/frame |
| Trace match | Run the same firmware, compare waveform to ngspice/bench capture |
| Object-shape stability | Verify no V8 deopt under rapid sample churn |
| Cross-channel | All 4+ channels on one synthetic waveform; simulate a known signal |
| WASM build | Same renderer draws correctly in the web target |

---

## 9. Design decisions summary

- **GPU-first rendering**: per-channel wgpu texture, 60 fps floor.
- **Trigger as the defining control**: Auto fills the screen until a real edge.
- **Badges + FFT as a toggle panel**, not a separate instrument.
- **Data-driven**: measurements computed from the sampled buffer, not the GPU.
- **Object-shape stability**: flat, fixed-layout buffers; single transformation layer.
