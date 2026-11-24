# Arduino-ESP32 and ESP-IDF MVP demos

Status: in progress. Tracks [#60](https://github.com/sjnorval/Ohmforge/issues/60). **D1 (ESP-IDF blink) implemented** (`firmware/esp32c3-idf-blinky`, `tests/idf_blinky.rs`); D2–D5 remain.
Date: 2026-11-23 (updated same day for D1).
Related: [`ROADMAP.md`](../ROADMAP.md) (P2 MVP demo exit; first-milestone step 8); [`firmware/README.md`](../../firmware/README.md); [`editor-integration.md`](editor-integration.md) (I6 Arduino board package is a different deliverable); [`AGENTS.md`](../../AGENTS.md) §3c (ROM/bootloader fetch rules); [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §8 (firmware loading), §13.1 (licensing).

The four MVP behaviours already run from **unmodified esp-hal** apps under `firmware/` (blink, button poll/irq, pot dimmer, LED fade). Release 0.1 also asks for the same behaviours from **Arduino-ESP32** and **ESP-IDF** firmware, so the claim “unmodified ESP32-C3 ELF (Arduino / ESP-IDF / esp-hal)” is true for each demo, not only for Rust.

## 1. Current state

| Piece | Where it lives | Status |
|---|---|---|
| Bare-metal / XIP blinky | `firmware/esp32c3-blinky`, `esp32c3-xip-blinky` | Done; zig-built, committed ELFs |
| esp-hal blinky | `firmware/esp32c3-esp-hal-blinky` | Done (#21); ROM + ESP-IDF bootloader path |
| esp-hal button (poll + irq) | `firmware/esp32c3-esp-hal-button` | Done (#22) |
| esp-hal ADC/PWM (dimmer + fade) | `firmware/esp32c3-esp-hal-adc-pwm` | Done (#24) |
| Desktop demos | `of-ui` (blink, dimmer, fade, button) | Drive the esp-hal (or bare-metal blink) images |
| Real ROM + espflash bootloader | Fetched by `scripts/fetch-esp-rom-elfs.*`, never committed | Required for the same boot path as esp-hal tests |
| Arduino-ESP32 / ESP-IDF demo apps | `firmware/esp32c3-idf-blinky` (D1); rest — | **In progress (#60)** |
| Arduino IDE board package (upload any sketch) | [`editor-integration.md`](editor-integration.md) I6 | P4; **not** this issue |

Roadmap step 8 still says Arduino/ESP-IDF blink is “deferred: needs an ESP-IDF or Arduino toolchain in CI”. This note un-defers the **design** and records how CI and licensing will work so implementation can start.

## 2. Goals and non-goals

Goals:

- Each of the four MVP behaviours runs from an **Arduino-ESP32** build and from an **ESP-IDF** build, not only from esp-hal.
- Apps use the **stock** Arduino-ESP32 / ESP-IDF APIs and the **same boot path** as the esp-hal demos: real ROM → espflash’s ESP-IDF second-stage bootloader → app in flash (partition table at `0x8000`, app at `0x10000` unless a toolchain requires otherwise and tests document it).
- **Pin and circuit parity** with the esp-hal demos (section 4), so the same co-simulation fixtures and desktop circuits apply.
- **Reproducible committed flash images** (and rebuild scripts), in the same spirit as `esp32c3-esp-hal-*`: CI rebuilds and fails if the bytes drift.
- ROM and bootloader blobs stay **fetched, never committed**.

Non-goals:

- Shipping the Arduino-ESP32 core, ESP-IDF, or their GPL/LGPL toolchains inside the Ohmforge repo or release (see §13.1 and editor-integration §5b).
- The Arduino IDE / VS Code **upload** workflow for arbitrary user sketches (that is I6 / I5 in P4).
- MicroPython, PlatformIO as a third first-class tree (PlatformIO may be used later as a *build helper* if it simplifies CI; it is not a third demo family for #60).
- Changing the desktop demo picker to prefer Arduino/IDF images (tests prove the ELFs; the UI may keep loading esp-hal until a follow-up).

## 3. What “unmodified” means here

Same meaning as for esp-hal:

- The app is ordinary firmware someone would flash to a real ESP32-C3 DevKit: Arduino `setup`/`loop` or ESP-IDF `app_main`, stock HAL/driver calls, stock bootloader and app image format.
- We do **not** strip the RTOS, replace the bootloader, or call into a private Ohmforge ABI.
- The **source** of each demo is still ours (AGPL-3.0-or-later sketches / IDF components), just as the esp-hal apps are ours. “Unmodified” refers to the **framework and boot path**, not to forbidding our own demo logic.

## 4. Demo matrix (parity with esp-hal)

| Behaviour | Pins / peripherals | esp-hal reference | Arduino sketch intent | ESP-IDF app intent |
|---|---|---|---|---|
| Blink | GPIO8 out; UART0 banner; ~500 µs half-period (or documented equivalent) | `esp32c3-esp-hal-blinky` | `digitalWrite` / `delayMicroseconds` (or `esp_timer`) on GPIO8; `Serial` banner | `gpio` + delay or `esp_timer`; `uart` / `esp_rom_printf` banner |
| Button poll | GPIO9 in (pull-up, pressed = low), GPIO8 LED; print on change | `button_poll.bin` | `pinMode` INPUT_PULLUP + digitalRead loop | `gpio` poll + pull-up |
| Button irq | GPIO9 falling edge → toggle GPIO8; prefer sleep/`wfi`-equivalent between edges | `button_irq.bin` | `attachInterrupt` | GPIO ISR service |
| Pot dimmer | GPIO2 = ADC1 ch2; GPIO8 = LEDC ~1 kHz; print mV / duty | `pot_dimmer.bin` | `analogRead` + `ledcWrite` | `adc_oneshot` + `ledc` |
| LED fade | GPIO8 LEDC hardware fade ~5 kHz | `led_fade.bin` | `ledcFade` / fade API if available in the pinned core; else timed duty steps matching the esp-hal envelope | `ledc` fade |

Banner strings should be distinct per family so tests can tell them apart, for example `ohmforge arduino blinky` and `ohmforge idf blinky`. Timing may match the esp-hal periods within the same tolerances the existing co-sim tests already use (edge lag, LED current bands).

Folder layout (proposed):

```text
firmware/
  esp32c3-arduino-blinky/          # or one workspace with multiple sketches
  esp32c3-arduino-button/
  esp32c3-arduino-adc-pwm/
  esp32c3-idf-blinky/
  esp32c3-idf-button/
  esp32c3-idf-adc-pwm/
```

Exact names can collapse (one Arduino project with multiple `.ino` / targets, one IDF project with multiple `main` apps) if that keeps CI simpler; the table above is the behavioural contract.

## 5. Toolchains, pinning and CI

### 5a. Versions (pin in the design before coding)

Propose pinning (bump only with a deliberate PR):

| Tool | Proposed pin | Notes |
|---|---|---|
| ESP-IDF | v5.3.2 or v5.4.x (decide at implementation from “known good with ESP32-C3 + LEDC fade”) | Install via Espressif’s GitHub Action / `esp-idf` export script; do not vendor the tree |
| Arduino-ESP32 core | 3.x line that targets IDF 5.x (match the pin above where possible) | Installed with `arduino-cli core install esp32:esp32@…` |
| `arduino-cli` | Current stable at implementation time, version-pinned in the build script | |
| Host compilers | Whatever those installs pull (riscv32-esp-elf, etc.) | Never committed |

Record the final pins in each folder’s `README` / `build.sh` and in `firmware/README.md`.

### 5b. Build outputs

Mirror esp-hal:

- Commit **`partitions.bin`** (or shared) and **app flash images** (`.bin`) used by tests.
- Do **not** commit the Arduino core, ESP-IDF checkout, or intermediate `build/` trees.
- `build.sh` (and `.ps1` where useful) must be reproducible enough that CI can rebuild and `cmp` against the committed images (reset file mode to 644; pin dates if the image embeds a build timestamp).

### 5c. CI strategy

Roadmap previously deferred this for lack of a toolchain in CI. Plan:

1. **Linux CI job** (primary): install pinned ESP-IDF (Espressif action or scripted clone + `install.sh` + `export.sh`) and `arduino-cli` + `esp32:esp32@pin`; run each `build.sh`; fail on byte/mode drift.
2. **Tests** (`of-soc-esp32c3`): same pattern as `tests/esp_hal_*.rs` — skip without ROM unless `OF_REQUIRE_ESP_ROM` is set; boot through real ROM + fetched bootloader; assert banners, GPIO/ADC/PWM behaviour against the existing circuit fixtures where practical.
3. **Windows/macOS**: optional rebuild jobs later; until then, Linux CI is the rebuild gate (as with other heavy firmware jobs if needed). Document that contributors rebuild on Linux or a container.

If full IDF install proves too slow for every PR, keep a **nightly / labeled** rebuild workflow and still run the **simulator tests** on every PR against the committed bins (same split as “committed ELF + optional rebuild” for zig firmware). Prefer full rebuild on every PR if wall time stays acceptable (&lt; ~15 minutes extra).

## 6. Licensing

| Artefact | Licence |
|---|---|
| Our sketches / IDF `main` components | AGPL-3.0-or-later (same as other `firmware/`) |
| Arduino-ESP32 core (linked into the user’s/demo binary) | LGPL-2.1 — **not redistributed**; only appears inside built images the way hardware builds do ([`editor-integration.md`](editor-integration.md) §5b) |
| ESP-IDF | Apache-2.0 (framework) — **not vendored** in-tree |
| espflash bootloader / esp-rom-elfs ROM | Fetched, checksummed, never committed (existing rules) |

`cargo deny` and repo licence policy are unchanged: we do not add GPL-2.0-only or LGPL **dependencies** to the Rust workspace. Running `arduino-cli` / `idf.py` as external tools is the same class of use as ngspice and zig.

## 7. Tests and desktop

- New tests under `crates/of-soc-esp32c3/tests/` (or shared helpers next to `esp_hal_*`) that load the Arduino/IDF images and reuse circuit setup from the esp-hal tests where possible.
- Acceptance does **not** require the desktop picker to switch defaults; optional follow-up can offer “firmware family” once images exist.
- Document in `firmware/README.md` how to rebuild and which test binary maps to which demo.

## 8. Phases

| Phase | Deliverable | Acceptance |
|---|---|---|
| **D0** | This design note; ROADMAP / README / AGENTS / mdBook / `firmware/README` pointers | Merged; #60 stays open for implementation |
| **D1** | ESP-IDF blink (+ test) | Banner + GPIO8 edges through ROM/bootloader; CI rebuild of that app |
| **D2** | Arduino-ESP32 blink (+ test) | Same for Arduino image |
| **D3** | Button poll + irq (both families) | Co-sim with push-button fixture (parity with `esp_hal_button`) |
| **D4** | Dimmer + fade (both families) | Co-sim with pot/LED fixture (parity with `esp_hal_adc_pwm`) |
| **D5** | ROADMAP MVP boxes for Arduino/IDF; changelog | #60 closable |

D1 before D2 (or the reverse) is fine if one toolchain is easier to pin first; both families are required to close #60.

## 9. Relation to editor integration

| Deliverable | Issue / phase | Audience |
|---|---|---|
| Committed Arduino + IDF **MVP demo images** and simulator tests | #60 (P2) | CI and “does Ohmforge run stock frameworks?” |
| Arduino IDE board package / VS Code upload | I6 / I5 (P4) | End users flashing **their** sketches |

Closing #60 does not implement I6. I6 later benefits from the same partition/bootloader knowledge and from proven Arduino core pins.

## 10. Open questions (resolve during D1)

1. Exact ESP-IDF and Arduino-ESP32 version pins (section 5a).
2. One repo folder per app vs one multi-target project per family.
3. Whether LED fade on the pinned Arduino core exposes hardware fade APIs equivalent to esp-hal’s; if not, timed duty steps with the same outer envelope are acceptable if tests document it.
4. Whether desktop demos gain a firmware-family switch in 0.1 or only in a follow-up.

## 11. Out of scope reminders

- Do not edit [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) unless Stefan asks; this note and the roadmap are enough.
- Do not commit ROM, bootloader, ESP-IDF trees, or Arduino packages.
- Do not weaken AGPL-compatible dependency rules to make CI easier.
