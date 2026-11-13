---
title: d4 — Logic analyser: protocol decode panels
type: decision
domain: ux
status: stable
updated: 2026-11-03
source: Digilent (Analog Discovery 3) | NI (2026) | Keysight (16850)
confidence: high
tags: [design, logic-analyzer, protocol-decode, decoding, bus]
---

# d4 — Logic analyser: protocol decode panels

## Decision

The logic analyser shows a timing view (one line per channel) **plus tagged protocol-decode panels** (I²C hex, SPI transaction, UART text, 1-Wire, WS2812). Decode is the value, not the squares.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| Raw 16-channel squares only | ❌ Low signal | Burden on the editor; no protocol insight |
| Decoded bus view only (no timing) | ❌ Loses edge detail | Timing is where edge faults live |

## Evidence

- Consumer floor: 16 channels (Keysight 16850). Professional tools go 32–128, but 16 matches the plan.
- Decode is the differentiator: I²C/SPI/UART/CAN/I2S protocol groups, timing view + bus view in one place, cursors + search (Digilent; NI, 2026).
- VCD export for PulseView/GTKWave keeps the simulator interoperable with open EDA tooling.

## Trade-offs

- Decode panels multiply the surface area for bugs (parser state, timing tolerance). Start with I²C/SPI/UART (the three the ESP32 uses) and add 1-Wire/WS2812 later.
- Keep the timing view and decode panels linked so a selected decode field highlights its edges in the timing view.

## Status / next

Implement EVT: I²C + SPI + UART decode; edge and transaction triggers; pattern capture + find-next; VCD + CSV export. Add 1-Wire/WS2812 decoders in a later phase.
