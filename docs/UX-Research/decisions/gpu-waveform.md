---
title: d2 — Oscilloscope rendering: GPU per-channel (wgpu)
type: decision
domain: ux
status: stable
updated: 2026-11-03
source: phosphor-crt (n.d.) | webgl-plot (n.d.) | github.com — Rust desktop GUI ecosystem, measured (2026)
confidence: high
tags: [design, oscilloscope, gpu, wgpu, rendering]
---

# d2 — Oscilloscope rendering: GPU per-channel (wgpu)

## Decision

The live oscilloscope renders one textured line primitive per channel in **wgpu**, driven at 60 fps; the CPU egui canvas is not used as the waveform surface.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| `egui_plot` CPU canvas | ❌ Too slow at 4+ channels | DOM/immediate-mode gut-rendering cannot hit 60 fps at high sample rates for multiple channels |
| `slint` / `gpui` | ❌ Coordination | Strong but add a second render path we don't need yet |
| CPU canvas (skia/vello) | ❌ Over-engineering | A single-GPU texture path is simpler for this app |

## Evidence

- waveform draw is a **GPU line-primitive problem**: phosphor-crt (wgpu, false-colour, cross-platform incl. WASM) and webgl-plot (native WebGL, high update rate, Web Workers) both use this pattern for real-time multi-waveform displays.
- 60 fps is the floor: below that the waveform continuity the whole instrument is built on breaks.

## Trade-offs

- Adds a wgpu texture path on top of egui (two render surfaces to manage). Deliberate: we keep egui for the rest, and the GPU path is only for real-time channels.
- Web target uses the same renderer via WebGPU/WASM, so the desktop and WASM apps share the surface.

## Status / next

Implement: per-channel line-primitive in a wgpu texture; auto-scale on connect; trigger modes (auto/norm/edge); FFT toggle + spectrum panel; measurement badges (Vpp, frequency, rise time); cursor overlay. Measure on a metal-poor device before the web build.
