---
title: d7 — Serial monitor: lean console + queued host→device
type: decision
domain: ux
status: stable
updated: 2026-11-20
source: docs/superpowers/specs/2026-11-20-serial-monitor-design.md | DESIGN_AND_PLAN.md §9
confidence: high
tags: [design, serial-monitor, uart, instruments]
---

# d7 — Serial monitor: lean console + queued host→device

## Decision

Ship a reusable lean serial monitor (text log, sim-time stamps, clear/copy, baud, Send) with model in `of-instruments` and egui panel in `of-ui`. Host→device Send **queues** bytes and logs them; until SoC UART RX exists, bytes are not delivered to firmware and the UI says so.

## Alternatives considered

| Option | Verdict | Reason |
|---|---|---|
| Blink-only UART dump | ❌ | Not reusable across projects |
| Disable Send until RX | ❌ | Wrong mental model for interactive firmware |
| Full shell + monitor in one PR | ❌ | Couples d5 shell to the instrument |

## Trade-offs

- Users can type before RX works; honesty depends on the status line.
- Soft-front-panel placement waits on d5.

## Status / next

RX FIFO delivery is in: blink attaches `Delivery::Attached` and drains Send into `Esp32c3::push_uart_rx`. Baud-timed RX, RX interrupts, and hex/find are in this PR. Still to do: host the panel in the d5 shell.
