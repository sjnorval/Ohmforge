# Serial monitor v1 — design

**Date:** 2026-11-20  
**Status:** Approved for implementation planning  
**Phase:** P2 (ESP32-C3 MVP instruments)  
**Anchors:** `docs/DESIGN_AND_PLAN.md` §9; `docs/ROADMAP.md` (serial monitor); UX decisions d5 (shell), d6 (egui)

## 1. Goal

Ship a **reusable lean serial monitor** that any simulation consumer can feed, not a blink-demo-only panel. The first consumer is the existing desktop blink demo (`BlinkDemo`).

### Non-goals (v1)

- Hex view, find/filters, pause, plotter
- Soft-front-panel app shell (d5) — later PR
- Real SoC UART RX delivery into firmware
- Logic-analyser UART decode (d4) — separate instrument

## 2. Architecture

| Layer | Crate | Responsibility |
|---|---|---|
| Model | `of-instruments` | Byte log, host→device queue, baud, clear, send/drain API |
| View | `of-ui` | `SerialMonitorPanel` egui widget |
| Consumer | `of-ui` blink (first) | Map SoC UART0 TX → `push_rx`; replace the monospace UART dump |

`of-ui` already depends on `of-instruments`. The instruments crate stays free of egui and of SoC types so other projects (CLI, future WASM, non-C3 boards) can reuse the model.

### Data flow

```
SoC UART0 TX ──push_rx──► SerialMonitor log (DeviceToHost)
User Send     ──try_send──► log (HostToDevice) + pending_tx queue
Future SoC RX ◄── drain_tx_queue ── pending_tx
```

Timestamps are **simulation time** (`of_core::SimTime`), never wall clock.

## 3. Model (`of_instruments::serial`)

### Types

- `Dir`: `DeviceToHost` | `HostToDevice` (host’s view of RX / TX)
- `Entry`: `{ at: SimTime, dir: Dir, data: Vec<u8> }`
- `Delivery`: `QueuedOnly` (v1 default). An attached/draining mode is reserved for when UART RX exists; v1 always reports queued-only status.
- `SerialMonitor`: entries, `pending_tx`, optional baud, delivery mode, byte/entry cap
- `SendError`: at least `Empty` (ignore empty Send in the UI; API may still return it)

### API

| Method | Behaviour |
|---|---|
| `push_rx(at, bytes)` | Append one device→host `Entry` with the given bytes (consumers choose chunking) |
| `try_send(at, bytes)` | On non-empty input: append host→device entry and extend `pending_tx`; return `Ok` under `QueuedOnly` |
| `drain_tx_queue()` | Take pending host→device bytes (for future SoC RX); blink does not drain in v1 |
| `clear()` | Clear log and pending queue |
| `set_baud(Option<f64>)` | Metadata for the header |
| `entries()` / display helpers | For the panel and unit tests |
| `delivery()` | Drives the status string |

**Cap:** keep at most **64 KiB** of payload bytes across all entries (sum of `data` lengths). When exceeded, drop oldest entries first until under the cap. Document the constant in the module.

**Independence:** no dependency on `of-soc-esp32c3`. Consumers map their own capture format into `push_rx`. Blink walks `Esp32c3::uart_bytes()`, keeps a cursor of how many bytes were already pushed, and feeds the delta as one `push_rx` per new `UartByte` (single-byte chunks). Replay/reset must `clear()` and reset the cursor.

**Line display:** the panel lossy-UTF-8-decodes the byte stream. For each direction it buffers into lines on `\n`; a log row uses the **timestamp of the first byte of that line**. A trailing partial line (no `\n` yet) is shown as a live last row.

## 4. View (`of_ui::SerialMonitorPanel`)

Visual language matches the blink demo panels (dark card fill ≈ `#161a20`, teal accents, monospace log).

### Layout (top → bottom)

1. **Header** — title (default `UART0`, overridable) · baud (`115200` or `—`) · Clear · Copy  
2. **Log** — monospace `ScrollArea`; auto-scroll when new device→host data arrives unless the user has scrolled up  
   - Line shape: `[0.125 ms] ← text` / `[1.200 ms] → text`  
   - Muted timestamps; `←` = device→host, `→` = host→device  
3. **Status** — when `Delivery::QueuedOnly`: *Send queues bytes — UART RX not modelled yet*  
4. **Input** — text field + Send; Enter sends; empty input is a no-op; Send calls `try_send(sim_now, bytes)` with a trailing newline (typical serial-line behaviour)

### Blink wiring

- Replace the current UART monospace dump in `blink.rs` with `SerialMonitorPanel`.
- Own a `SerialMonitor` beside the sim; on advance/tick, sync new UART bytes and baud when available.
- Reset / Replay: rebuild or `clear()` the monitor with the sim.
- Schematic and LED-current panels stay unchanged.

## 5. Testing

- **`of-instruments` unit tests:** `push_rx`, `try_send` (including empty), `drain_tx_queue`, `clear`, baud, delivery status, history cap.
- **Blink sim test:** keep asserting the firmware banner appears (via monitor text and/or existing `uart_text()`).
- **egui:** no deep widget unit tests in v1; model tests own the behaviour.

## 6. Docs and rollout

- `CHANGELOG.md` — Added: reusable serial monitor (lean console; queued Send until UART RX).
- `docs/ROADMAP.md` — serial monitor marked partly done: TX console + queued send; RX delivery still open.
- UX decision **d7** — “serial monitor: lean console + queued host→device” — recorded under `docs/UX-Research/decisions/` and indexed in that folder’s README.
- One feature branch / PR: model + panel + blink consumer. **No** SoC RX changes.

### Follow-ups (explicitly out of this PR)

1. SoC UART RX hooked to `drain_tx_queue` for interactive firmware  
2. Hex toggle / find / filters  
3. Soft-front-panel shell (d5) hosting the panel as an instrument  
4. Serial plotter  

## 7. Success criteria

- Another crate can depend on `of-instruments` and drive a serial log without importing `of-ui` or blink.
- Blink demo shows a scrollable, clearable console with sim-time stamps and a Send box that queues with an honest status line.
- `cargo test -p of-instruments` and existing blink/GUI checks stay green.
