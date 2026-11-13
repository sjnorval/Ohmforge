---
title: d6 — UI toolkit: keep egui/eframe 0.37
type: decision
domain: ux
status: stable
updated: 2026-11-03
source: github.com — The Rust desktop GUI ecosystem, measured (2026) | github.com — egui downloads
confidence: high
tags: [design, toolchain, egui, iced, slint, gpui]
---

# d6 — UI toolkit: keep egui/eframe 0.37

## Decision

Stay on **egui/eframe 0.37** (immediate mode + `egui_plot` for charts). Do not switch to `iced`, `slint`, or `gpui`.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| `iced` 0.14 | ❌ | Clean Elm seam, but slower ecosystem, no upstream AccessKit, Wayland/NVIDIA issues |
| `slint` 1.19 | ❌ | GPLv3/dual, embedded-MCU-only profile, restrictive attribution; not what's needed here |
| `gpui` | ❌ | Zed-like hybrid, GPU-rendered; strong but higher learning curve and no native chart path |

## Evidence

- GitHub — "The Rust desktop GUI ecosystem, measured" (2026): egui has the largest downloads (30.3M, ~10× iced and slint), biggest ecosystem, best docs, AccessKit, and a native chart path (`egui_plot`).
- egui immediate mode is ideal for instruments (repaint at 60 fps, pure Rust, one codebase for desktop + WASM).

## Trade-offs

- egui immediate mode requires custom canvas/interaction code (which is precisely what we're writing for the schematic editor — see d1).
- Breaking releases every 1–3 months; pin a toolchain version and treat upgrades as tracked work.

## Status / next

Keep. Only revisit if the schematic canvas becomes the bottleneck (then evaluate a dedicated figure engine) or if a native chart path becomes a must-have (watch `gpui` maturity).
