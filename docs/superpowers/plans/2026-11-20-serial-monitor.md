# Serial Monitor v1 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Ship a reusable lean serial-monitor model in `of-instruments` and an egui panel in `of-ui`, wired into the blink demo as the first consumer (queued Send until SoC UART RX exists).

**Architecture:** UI-free `SerialMonitor` owns the byte log, host→device queue, baud, and 64 KiB history cap. `SerialMonitorPanel` draws that model. `BlinkDemo` pushes SoC UART0 TX deltas into the model and replaces the monospace UART dump. No soft-front-panel shell and no SoC RX changes in this PR.

**Tech Stack:** Rust edition 2024, `of-core::SimTime`, egui 0.37 (behind `of-ui` `gui` feature), thiserror for `SendError`.

**Spec:** `docs/superpowers/specs/2026-11-20-serial-monitor-design.md`

## Global Constraints

- AGPL-3.0-or-later; no new copyleft deps; run `cargo deny check` if available.
- No `unwrap()` in library code outside tests; every public item gets a doc comment (`missing_docs` warn).
- `of-instruments` must stay free of egui and of `of-soc-esp32c3`.
- Simulation timestamps only (`SimTime`); never wall clock.
- Do not commit without Stefan’s approval (leave working tree ready; propose Conventional Commits + `Signed-off-by`).
- Load MSVC Build Tools env on Windows before cargo (see `AGENTS.md` §9).

## File map

| Path | Role |
|---|---|
| `crates/of-instruments/src/lib.rs` | Export `serial` module |
| `crates/of-instruments/src/serial.rs` | Model + unit tests + display-line helper |
| `crates/of-instruments/Cargo.toml` | Add `thiserror` |
| `crates/of-ui/src/lib.rs` | Export panel behind `gui` |
| `crates/of-ui/src/serial.rs` | `SerialMonitorPanel` egui widget |
| `crates/of-ui/src/blink.rs` | Own monitor, sync UART, draw panel |
| `crates/of-ui/src/blink/simulation.rs` | Helpers to read `uart_bytes` + baud (optional small API) |
| `docs/UX-Research/decisions/serial-monitor.md` | Decision d7 |
| `docs/UX-Research/decisions/README.md` | Index d7 |
| `CHANGELOG.md` | Added line |
| `docs/ROADMAP.md` | Mark serial monitor partly done |

---

### Task 1: `SerialMonitor` model in `of-instruments`

**Files:**
- Create: `crates/of-instruments/src/serial.rs`
- Modify: `crates/of-instruments/src/lib.rs`
- Modify: `crates/of-instruments/Cargo.toml`
- Test: unit tests at the bottom of `serial.rs`

**Interfaces:**
- Consumes: `of_core::SimTime`
- Produces:
  - `pub enum Dir { DeviceToHost, HostToDevice }`
  - `pub struct Entry { pub at: SimTime, pub dir: Dir, pub data: Vec<u8> }`
  - `pub enum Delivery { QueuedOnly }`
  - `pub enum SendError { Empty }` (thiserror)
  - `pub struct SerialMonitor` with:
    - `pub fn new() -> Self` / `Default`
    - `pub const MAX_PAYLOAD_BYTES: usize = 64 * 1024`
    - `pub fn push_rx(&mut self, at: SimTime, bytes: &[u8])`
    - `pub fn try_send(&mut self, at: SimTime, bytes: &[u8]) -> Result<(), SendError>`
    - `pub fn drain_tx_queue(&mut self) -> Vec<u8>`
    - `pub fn clear(&mut self)`
    - `pub fn set_baud(&mut self, baud: Option<f64>)`
    - `pub fn baud(&self) -> Option<f64>`
    - `pub fn delivery(&self) -> Delivery`
    - `pub fn entries(&self) -> &[Entry]`
    - `pub fn pending_tx(&self) -> &[u8]`
    - `pub fn display_lines(&self) -> Vec<(SimTime, Dir, String)>` — lossy UTF-8, split on `\n`, timestamp = first byte of each line; trailing partial line included
    - `pub fn text_device_to_host(&self) -> String` — concatenate all DeviceToHost payloads (for blink banner asserts)

- [ ] **Step 1: Add `thiserror` dependency**

In `crates/of-instruments/Cargo.toml`:

```toml
[dependencies]
of-core.workspace = true
thiserror.workspace = true
```

- [ ] **Step 2: Write failing tests first**

Replace `crates/of-instruments/src/lib.rs` contents with a module export (keep a short crate docs comment), and create `serial.rs` with tests only first (types stubbed so the file compiles after Step 3). Prefer writing the full test module now:

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use of_core::SimTime;

    #[test]
    fn push_rx_records_device_to_host() {
        let mut m = SerialMonitor::new();
        m.push_rx(SimTime::from_us(100), b"hi");
        assert_eq!(m.entries().len(), 1);
        assert_eq!(m.entries()[0].dir, Dir::DeviceToHost);
        assert_eq!(m.entries()[0].data, b"hi");
        assert_eq!(m.text_device_to_host(), "hi");
    }

    #[test]
    fn try_send_empty_errors_and_leaves_state() {
        let mut m = SerialMonitor::new();
        assert!(matches!(m.try_send(SimTime::ZERO, b""), Err(SendError::Empty)));
        assert!(m.entries().is_empty());
        assert!(m.pending_tx().is_empty());
    }

    #[test]
    fn try_send_queues_and_logs_host_to_device() {
        let mut m = SerialMonitor::new();
        m.try_send(SimTime::from_us(1), b"help\n").unwrap();
        assert_eq!(m.pending_tx(), b"help\n");
        assert_eq!(m.entries()[0].dir, Dir::HostToDevice);
        assert_eq!(m.delivery(), Delivery::QueuedOnly);
        assert_eq!(m.drain_tx_queue(), b"help\n");
        assert!(m.pending_tx().is_empty());
    }

    #[test]
    fn clear_resets_log_and_queue_keeps_baud() {
        let mut m = SerialMonitor::new();
        m.set_baud(Some(115_200.0));
        m.push_rx(SimTime::ZERO, b"x");
        m.try_send(SimTime::ZERO, b"y").unwrap();
        m.clear();
        assert!(m.entries().is_empty());
        assert!(m.pending_tx().is_empty());
        assert_eq!(m.baud(), Some(115_200.0));
    }

    #[test]
    fn history_cap_drops_oldest_payload() {
        let mut m = SerialMonitor::new();
        // Two chunks that together exceed the cap when a third is added.
        let big = vec![b'a'; SerialMonitor::MAX_PAYLOAD_BYTES / 2 + 1];
        m.push_rx(SimTime::from_us(1), &big);
        m.push_rx(SimTime::from_us(2), &big);
        m.push_rx(SimTime::from_us(3), b"z");
        let total: usize = m.entries().iter().map(|e| e.data.len()).sum();
        assert!(total <= SerialMonitor::MAX_PAYLOAD_BYTES);
        assert_eq!(m.entries().last().unwrap().data, b"z");
    }

    #[test]
    fn display_lines_split_on_newline_with_first_byte_timestamp() {
        let mut m = SerialMonitor::new();
        m.push_rx(SimTime::from_us(10), b"oh");
        m.push_rx(SimTime::from_us(11), b"m\n");
        m.push_rx(SimTime::from_us(20), b"forge");
        let lines = m.display_lines();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].0, SimTime::from_us(10));
        assert_eq!(lines[0].1, Dir::DeviceToHost);
        assert_eq!(lines[0].2, "ohm");
        assert_eq!(lines[1].0, SimTime::from_us(20));
        assert_eq!(lines[1].2, "forge");
    }
}
```

- [ ] **Step 3: Run tests — expect compile/link failure**

```powershell
cargo test -p of-instruments -- --nocapture
```

Expected: FAIL — `SerialMonitor` / module not found or unresolved imports.

- [ ] **Step 4: Implement `serial.rs`**

`crates/of-instruments/src/lib.rs`:

```rust
//! Oscilloscope, logic analyser, protocol decoders, multimeter, PSU.
//!
//! Serial monitor model lives in [`serial`]; egui panels live in `of-ui`.

pub mod serial;

pub use serial::{Delivery, Dir, Entry, SendError, SerialMonitor};
```

Implement `serial.rs` with the types and methods from **Interfaces**. Behaviour notes:

- `push_rx`: if `bytes` is empty, no-op. Else push one `Entry`, then call internal `trim_to_cap()`.
- `try_send`: empty → `Err(SendError::Empty)`; else append `HostToDevice` entry, extend `pending_tx`, `trim_to_cap()` (cap applies to log entries’ payload sum; `pending_tx` is separate and unbounded in v1 — keep it small via UI lines).
- `trim_to_cap`: while sum of `entries[].data.len() > MAX_PAYLOAD_BYTES`, `entries.remove(0)`.
- `display_lines`: walk entries **in order**, but buffer **per direction separately** (do not merge RX and TX into one line). For each entry, decode lossy and append to that direction’s buffer; on `\n`, emit a line with the stored start `SimTime` and clear buffer; after all entries, emit any non-empty partial buffer per direction (order: emit completed lines in stream order; for partials, emit DeviceToHost partial then HostToDevice partial if both exist — document this in the method docs). Simpler acceptable algorithm that still passes the test above: single pass over entries in order, maintaining one buffer + start time **for the current direction**, flushing when direction changes or `\n` seen.
- `text_device_to_host`: concat `data` for `Dir::DeviceToHost` only.
- `clear`: clear `entries` and `pending_tx`; leave `baud` and `delivery` alone.
- `delivery`: always `Delivery::QueuedOnly` in v1.
- Doc-comment every public item.

Suggested flush-on-direction-change sketch for `display_lines`:

```rust
pub fn display_lines(&self) -> Vec<(SimTime, Dir, String)> {
    let mut out = Vec::new();
    let mut buf = String::new();
    let mut start = SimTime::ZERO;
    let mut dir: Option<Dir> = None;

    let flush_line = |out: &mut Vec<_>, buf: &mut String, start: SimTime, d: Dir| {
        if let Some(nl) = buf.find('\n') {
            // handled in loop below instead
        }
        let _ = (out, buf, start, d);
    };
    let _ = flush_line;

    for e in &self.entries {
        if dir != Some(e.dir) {
            if let Some(d) = dir {
                if !buf.is_empty() {
                    out.push((start, d, std::mem::take(&mut buf)));
                }
            }
            dir = Some(e.dir);
            start = e.at;
        }
        let chunk = String::from_utf8_lossy(&e.data);
        for ch in chunk.chars() {
            if buf.is_empty() {
                start = e.at;
            }
            if ch == '\n' {
                out.push((start, e.dir, std::mem::take(&mut buf)));
            } else {
                buf.push(ch);
            }
        }
    }
    if let Some(d) = dir {
        if !buf.is_empty() {
            out.push((start, d, buf));
        }
    }
    out
}
```

Fix the sketch so the first byte of a new line after `\n` sets `start` from the entry that provided that byte (the loop above already resets `start` when `buf.is_empty()` before pushing a char). Strip `\r` if you want; not required by tests.

- [ ] **Step 5: Run tests — expect pass**

```powershell
cargo test -p of-instruments -- --nocapture
```

Expected: all `serial` tests PASS.

- [ ] **Step 6: Propose commit (do not commit unless Stefan approves)**

```
feat(instruments): add SerialMonitor model with queued TX

UI-free byte log, 64 KiB cap, and display_lines for the lean console.
```

```powershell
git add crates/of-instruments/src/lib.rs crates/of-instruments/src/serial.rs crates/of-instruments/Cargo.toml
# git commit -s -m "..."   # only after approval
```

---

### Task 2: `SerialMonitorPanel` in `of-ui`

**Files:**
- Create: `crates/of-ui/src/serial.rs`
- Modify: `crates/of-ui/src/lib.rs`

**Interfaces:**
- Consumes: `of_instruments::{SerialMonitor, Dir, Delivery}`, `of_core::SimTime`, `egui`
- Produces: `pub struct SerialMonitorPanel; impl SerialMonitorPanel { pub fn show(ui: &mut Ui, monitor: &mut SerialMonitor, title: &str, sim_now: SimTime) }`

- [ ] **Step 1: Export module behind `gui`**

`crates/of-ui/src/lib.rs` should look like:

```rust
//! egui views and instrument panels, enabled by the `gui` feature.

#[cfg(feature = "gui")]
mod blink;
#[cfg(feature = "gui")]
mod serial;

#[cfg(feature = "gui")]
pub use blink::BlinkDemo;
#[cfg(feature = "gui")]
pub use serial::SerialMonitorPanel;
```

- [ ] **Step 2: Implement the panel**

Create `crates/of-ui/src/serial.rs`. Match blink panel chrome (`Color32::from_rgb(22, 26, 32)`, corner radius 8, margin 14). Behaviour:

1. Header row: `title` (strong) · baud as integer if present else `—` · `Clear` button · `Copy` button (copies a plain text dump of `display_lines()` via `ui.ctx().copy_text(...)`).
2. `ScrollArea::vertical()` with `id_salt("serial_monitor_log")`, min height ~140. For each `(at, dir, text)` from `monitor.display_lines()`:
   - Format time as milliseconds with 3 decimals: `format!("[{:.3} ms]", at.as_secs_f64() * 1e3)`
   - Arrow: `←` for `DeviceToHost`, `→` for `HostToDevice`
   - `ui.monospace(format!("{time} {arrow} {text}"))`
3. If `monitor.delivery() == Delivery::QueuedOnly`, weak status label: `Send queues bytes — UART RX not modelled yet`
4. Input row: `TextEdit::singleline(&mut draft)` stored in `SerialMonitorPanel` **or** passed as `&mut String` draft owned by the caller. Prefer the panel owning draft state:

```rust
#[derive(Default)]
pub struct SerialMonitorPanel {
    draft: String,
    /// Stick to bottom unless the user scrolled up.
    follow: bool,
}

impl SerialMonitorPanel {
    pub fn show(&mut self, ui: &mut Ui, monitor: &mut SerialMonitor, title: &str, sim_now: SimTime) {
        // ...
    }
}
```

Auto-scroll: use `ScrollArea` stick-to-bottom pattern (egui `scroll_offset` / `ScrollArea::vertical().animated(false)` + track max scroll). Minimal acceptable v1: always scroll to bottom when `follow` is true; set `follow = false` if the user scrolls up (detect via `ScrollArea` output if available in 0.37); Clear and new RX can set `follow = true`. If scroll detection is awkward in 0.37, **always auto-scroll** in v1 and note it in the PR — prefer always-follow over complex scroll hacks.

Send: on button click or Enter (`TextEdit` returned `response.lost_focus()` + enter, or `ui.input` key), if draft non-empty after trim check:

```rust
let mut payload = self.draft.clone();
if !payload.ends_with('\n') {
    payload.push('\n');
}
let _ = monitor.try_send(sim_now, payload.as_bytes());
self.draft.clear();
```

Clear button calls `monitor.clear()` and clears `draft`.

- [ ] **Step 3: Compile-check GUI crate**

```powershell
cargo check -p of-ui --features gui
cargo check -p of-app --features gui
```

Expected: success.

- [ ] **Step 4: Propose commit**

```
feat(ui): add SerialMonitorPanel lean console
```

---

### Task 3: Wire blink demo as first consumer

**Files:**
- Modify: `crates/of-ui/src/blink.rs`
- Modify: `crates/of-ui/src/blink/simulation.rs` (add byte/baud accessors)
- Test: existing `simulation.rs` tests + adjust if UART assertion path changes

**Interfaces:**
- Consumes: Task 1 + Task 2 APIs; `Esp32c3Engine` / `uart_bytes()` / `bus.uart0.baud(80_000_000)`
- Produces: BlinkDemo shows the panel; banner still observable

- [ ] **Step 1: Expose raw UART bytes and baud from `Blink`**

In `simulation.rs`, add:

```rust
pub(super) fn uart_bytes(&mut self) -> &[of_soc_esp32c3::UartByte] {
    &self.kernel
        .engine_as::<Esp32c3Engine>(self.cpu)
        .expect("registered ESP32-C3")
        .soc
        .uart_bytes()
}

pub(super) fn uart_baud(&mut self) -> Option<f64> {
    const APB_HZ: u64 = 80_000_000;
    self.kernel
        .engine_as::<Esp32c3Engine>(self.cpu)
        .expect("registered ESP32-C3")
        .soc
        .bus
        .uart0
        .baud(APB_HZ)
}
```

Keep `uart()` / `uart_text` path for the existing unit test (`assert_eq!(sim.uart(), "ohmforge blinky\n")`).

- [ ] **Step 2: Hold monitor + panel + cursor on `BlinkDemo`**

```rust
pub struct BlinkDemo {
    sim: Result<Blink, Box<dyn std::error::Error>>,
    playing: Option<(f64, f64)>,
    monitor: of_instruments::SerialMonitor,
    serial_panel: crate::SerialMonitorPanel,
    uart_cursor: usize,
}
```

On `new` / `restart`: `monitor.clear()` or replace with `SerialMonitor::new()`, `uart_cursor = 0`, reset panel draft via `serial_panel = Default::default()`.

- [ ] **Step 3: Sync helper called from `ui` after `tick`**

```rust
fn sync_serial(&mut self) {
    let Ok(sim) = &mut self.sim else { return };
    self.monitor.set_baud(sim.uart_baud());
    let bytes = sim.uart_bytes().to_vec(); // or sync without clone if lifetimes allow
    while self.uart_cursor < bytes.len() {
        let b = bytes[self.uart_cursor];
        self.monitor.push_rx(b.at, &[b.byte]);
        self.uart_cursor += 1;
    }
}
```

Avoid holding conflicting borrows: read baud and copy the new slice of bytes first, then push.

- [ ] **Step 4: Replace the UART dump UI**

In `instruments(...)`, remove the monospace `uart` panel block. Instead, from `BlinkDemo::ui` after building instruments (or pass `&mut self.monitor` / panel into `instruments`):

```rust
self.sync_serial();
// ... schematic + scope ...
let now = match &self.sim {
    Ok(sim) => of_core::SimTime::from_secs_f64(sim.time()),
    Err(_) => of_core::SimTime::ZERO,
};
panel_chrome_or_serial.show(...);
self.serial_panel.show(ui, &mut self.monitor, "UART0", now);
```

Reuse the same outer `panel()` frame inside `SerialMonitorPanel` so double-framing does not happen — either panel draws its own frame (preferred, as in Task 2) or blink wraps it once.

- [ ] **Step 5: Extend / keep tests**

Keep `desktop_blink_advances_firmware_and_records_led_current` asserting `sim.uart()`. Optionally add:

```rust
#[test]
fn monitor_receives_blink_banner_when_synced() {
    let mut demo_monitor = of_instruments::SerialMonitor::new();
    let mut sim = Blink::new().unwrap();
    sim.advance(125e-6).unwrap();
    for b in sim.uart_bytes() {
        demo_monitor.push_rx(b.at, &[b.byte]);
    }
    assert_eq!(demo_monitor.text_device_to_host(), "ohmforge blinky\n");
}
```

- [ ] **Step 6: Run verification**

```powershell
cargo test -p of-instruments
cargo test -p of-ui --features gui
cargo check -p of-app --features gui
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
```

Expected: all green. Manually: `cargo run -p of-app --features gui` → Play → banner lines with timestamps; type in Send → host line appears; status mentions queued RX.

- [ ] **Step 7: Propose commit**

```
feat(ui): wire SerialMonitor into the blink demo
```

---

### Task 4: Docs (d7, changelog, roadmap)

**Files:**
- Create: `docs/UX-Research/decisions/serial-monitor.md`
- Modify: `docs/UX-Research/decisions/README.md`
- Modify: `CHANGELOG.md`
- Modify: `docs/ROADMAP.md`

- [ ] **Step 1: Write d7**

`docs/UX-Research/decisions/serial-monitor.md`:

```markdown
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

Hook `drain_tx_queue` to SoC UART RX; then hex/find; then host the panel in the d5 shell.
```

Index in `docs/UX-Research/decisions/README.md` table:

```markdown
| `d7` | Serial monitor: lean console + queued host→device | Stable |
```

- [ ] **Step 2: CHANGELOG**

Under `### Added`:

```markdown
- **Serial monitor (lean console):** reusable `of-instruments` model and `of-ui` panel (timestamps, clear/copy, baud, Send). Host→device bytes queue until UART RX is modelled; the blink demo is the first consumer.
```

- [ ] **Step 3: ROADMAP**

Under the instruments bullet (~line 103), change to something like:

```markdown
- [ ] egui schematic editor (place, wire, probe), oscilloscope, logic analyser with UART decode, serial monitor
  - [x] Read-only desktop blink demo (`of-ui` `BlinkDemo`, ...). Not the editor or the real oscilloscope (GPU waveform path, decision d2)
  - [~] Serial monitor: lean console in `of-instruments` / `of-ui` (TX log, queued Send); SoC UART RX delivery, hex/find, and soft-front-panel hosting still to do (d7)
```

Also under MVP definition instruments line (~126), note serial monitor partly done the same way if it reads as unchecked-only today.

- [ ] **Step 4: Propose commit**

```
docs: record serial monitor d7 and roadmap/changelog
```

- [ ] **Step 5: Final workspace gate before PR**

```powershell
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo check -p of-app --features gui
```

---

## Spec coverage checklist

| Spec requirement | Task |
|---|---|
| Model in `of-instruments` | 1 |
| `push_rx` / `try_send` / `drain_tx_queue` / clear / baud / QueuedOnly | 1 |
| 64 KiB cap | 1 |
| `display_lines` / lossy UTF-8 / newline split | 1 |
| No SoC dependency in instruments | 1 |
| egui lean console panel | 2 |
| Status string for queued delivery | 2 |
| Send adds trailing `\n` | 2 |
| Blink consumer + cursor sync | 3 |
| Banner still testable | 3 |
| d7 + CHANGELOG + ROADMAP | 4 |
| No SoC RX / no shell / no hex | — out of scope |

## Placeholder / consistency review

- Method names aligned across tasks: `try_send`, `drain_tx_queue`, `display_lines`, `text_device_to_host`, `SerialMonitorPanel::show`.
- Cap constant: `MAX_PAYLOAD_BYTES = 64 * 1024`.
- APB for baud: `80_000_000` (matches AGENTS.md APB note).
