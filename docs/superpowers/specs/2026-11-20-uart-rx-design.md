# UART0 RX FIFO (minimal interactive) — design

**Date:** 2026-11-20  
**Status:** Approved for implementation planning  
**Phase:** P2 (ESP32-C3 MVP)  
**Anchors:** `docs/ROADMAP.md` (UART baud timing and RX); d7 serial monitor; `docs/superpowers/specs/2026-11-20-serial-monitor-design.md`

## 1. Goal

Host→device bytes from the serial monitor (and tests) appear in UART0’s RX FIFO so firmware can read them via `UART_FIFO`. Delivery is **instant** (same simplification as TX capture). No baud timing and no RX interrupts in this slice.

### Non-goals

- Baud-timed RX bit delivery
- RX interrupts (`RXFIFO_FULL`, `RXFIFO_TOUT`, etc.)
- USB Serial/JTAG RX
- New echo firmware ELF (unit tests prove the FIFO; blinky itself does not read UART)

## 2. Architecture

```
User Send → SerialMonitor.try_send → pending_tx
Blink each frame → drain_tx_queue → Esp32c3::push_uart_rx → Uart RX FIFO
Firmware UART_FIFO read ← pop; STATUS.RXFIFO_CNT = depth
```

Public API: `Esp32c3::push_uart_rx(&[u8])` (tests and any consumer). Blink is the first UI caller. Kernel events are not required while RX is untimed.

## 3. SoC model (`of-soc-esp32c3`)

### `Uart`

- Add `rx: VecDeque<u8>` with **max depth 128** (ESP32-C3 UART FIFO size).
- `push_rx(&[u8])`: append bytes; if over capacity, **drop oldest** so the newest host input is kept; document in the module docs.
- `read(FIFO)`: pop one byte from the front, or `0` if empty.
- `STATUS`: keep the existing idle line bits; set `RXFIFO_CNT` (bits 9:0) to `rx.len() as u32`; `TXFIFO_CNT` remains 0.
- `new()` / reset path clears `rx`.

### Bus / SoC

- FIFO **writes** stay as today (TX capture into `uart_tx`).
- FIFO **reads** already go through `uart0.read` — no bus special-case once `Uart` owns RX.
- `Esp32c3::push_uart_rx(bytes: &[u8])` forwards to `bus.uart0.push_rx(bytes)`.
- Optional `SocBus::push_uart_rx` if it keeps call sites cleaner.

### Tests

- Push `b"ab"` → `STATUS.RXFIFO_CNT == 2` → FIFO reads `b'a'`, `b'b'` → empty / cnt 0.
- Push more than 128 bytes → length ≤ 128 and newest bytes retained.
- Reset clears the RX FIFO.

## 4. Serial monitor delivery (`of-instruments` / `of-ui`)

- Extend `Delivery` with `Attached` (keep `QueuedOnly`).
- `SerialMonitor::set_delivery(Delivery)` (default remains `QueuedOnly` for callers that have not wired RX).
- Panel status copy:
  - `QueuedOnly`: `Send queues bytes — UART RX not modelled yet`
  - `Attached`: `Send delivers to UART0 RX`

### Blink wiring

- On `new` / `restart`: `monitor.set_delivery(Delivery::Attached)`.
- Each `sync_serial` (or equivalent): `let pending = self.monitor.drain_tx_queue();` then `sim.push_uart_rx(&pending)`.
- Order: drain host→device into the SoC first, then sync device→host TX bytes into the log (existing cursor path).
- Expose `Blink::push_uart_rx` via `kernel.engine_as::<Esp32c3Engine>(...).soc.push_uart_rx(...)`.

Note: the bundled blinky ELF does not read UART; bytes sit in the FIFO until firmware that polls RX is loaded. The status line still correctly means “bytes reached the SoC RX FIFO.”

## 5. Docs and rollout

- `CHANGELOG.md` — Added: UART0 RX FIFO (instant); serial monitor Send delivers when attached.
- `docs/ROADMAP.md` — UART RX partly done: FIFO + STATUS count; baud timing and RX interrupts still open.
- `AGENTS.md` §3c — note RX FIFO push/pop and STATUS count.
- Update d7 “Status / next” to mark RX FIFO done; interrupts / baud timing remain.

## 6. Success criteria

- Unit tests prove push → cnt → FIFO pop.
- Blink with `Delivery::Attached` drains Send into `push_uart_rx` without panics.
- Panel status no longer claims RX is unmodelled when attached.
- `cargo test -p of-soc-esp32c3` (UART-related), `cargo test -p of-instruments`, `cargo test -p of-ui --features gui`, GUI clippy for touched crates stay green.
