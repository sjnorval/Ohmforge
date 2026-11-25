# MicroPython on the ESP32-C3

Status: proposed (design only, no code). MicroPython is a stated project goal and an accepted firmware format in [`DESIGN_AND_PLAN.md`](../DESIGN_AND_PLAN.md) §8, but it is not part of the P2 Arduino/ESP-IDF demo family (#60). This note gives it a phase.
Date: 2026-11-25.
Related: [`arduino-idf-mvp-demos.md`](arduino-idf-mvp-demos.md) (explicitly out of scope for #60); [`editor-integration.md`](editor-integration.md) (I2 virtual serial, so the REPL can be used like a serial port); [`ROADMAP.md`](../ROADMAP.md) P4 and P6.

The goal is to boot an unmodified official MicroPython image for the ESP32-C3 on the same ROM and flash path as C and Rust firmware, and to use the UART REPL against a co-simulated circuit. This is a firmware-loading and peripheral-coverage task, not a second simulator.

## 1. Current state

esp-hal and ESP-IDF blink images already boot through the real ROM and the ESP-IDF second-stage bootloader. Arduino-ESP32 blink is still to do under #60. Direct ELF boot and flash-image boot both exist. There is no MicroPython image, filesystem, or REPL test. The #60 design note correctly keeps MicroPython out of that MVP slice.

## 2. Goals and non-goals

Goals:

- An official ESP32-C3 MicroPython firmware image, unmodified, boots on the model through the same ROM and flash path used by the other images.
- The UART0 REPL works: a banner, a prompt, and enough of `machine.Pin` (and, where the model already has the peripheral, ADC or PWM) to drive the official blink or dimmer circuit.
- A documented way to put `main.py` (or equivalent) on the image — a virtual filesystem the official firmware already understands, or a pre-baked image built in CI from a tiny in-tree script — so an example can run without typing at the prompt.
- The same I2 virtual serial port (P3) can be used as the REPL, so the user’s usual serial monitor talks to MicroPython the way it would on a board.

Non-goals:

- Shipping a fork of MicroPython, or treating a rewritten subset of the language as “MicroPython”.
- Bundling a MicroPython source tree in the Ohmforge repository if that would confuse licensing or make releases heavy. Prefer a pinned, checksummed official image fetched by a script (never committed when redistribution is unclear), or a CI job that builds the official tree out of tree.
- Making MicroPython a third #60 demo family, or blocking 0.1 on it.
- A complete port of every MicroPython module on day one. Modules that need Wi-Fi, BLE, or unmodelled peripherals wait for those peripherals. The P4 slice is boot, REPL, and GPIO (then ADC/PWM if the image uses them in the official example).

## 3. User-facing behaviour

The user obtains or builds the official ESP32-C3 MicroPython image the way they would for a board, or opens an official Ohmforge example that already includes it. They load it like any other flash image, open the serial monitor, and see the REPL. In an official example, `main.py` already blinks the LED on the co-simulated circuit. From P3 I2 onwards, `esptool` / the editor serial monitor can talk to the same REPL over the virtual port.

## 4. Technical approach

- Treat the image as a flash payload: bootloader, partition table, application, and any littlefs/fat partition the official image expects. Extend the existing image and partition support rather than adding a side path.
- Identify the minimum SoC and flash behaviour the official image touches at boot (filesystem, USB-CDC versus UART0, partitions). Missing behaviour is a list in this note’s implementation issue, not something to stub silently: if a required peripheral is absent, the test fails with a clear log, and we add the model or document a supported official build option (for example UART REPL rather than USB).
- Tests: a CI job that boots the image, waits for the REPL banner at the stated baud, sends a Pin toggle, and checks the LED circuit, deterministic and offline.
- Licensing: record the MicroPython licence and the image’s origin in NOTICE or the fetch script comments. Do not relicense upstream sources. Example scripts we author are AGPL-3.0-or-later; docs and circuit files stay CC-BY-SA-4.0.

## 5. Phasing

- **P4 / 0.3:** ESP32-C3 only, after I2 so the REPL has a virtual serial port, and after the P2 Arduino/ESP-IDF demos are on `main`. One official example in the gallery. Needs the C3 flash and UART path already used by esp-hal and ESP-IDF.
- **P6 / 1.0:** the same image family on ESP32-S3 and ESP32-C6 when those SoCs are supported, if official images exist for them.

## 6. Acceptance criteria

- An unmodified official C3 image boots through the real ROM path used by the other firmware families, in CI, without a network.
- The UART REPL banner appears, and a Pin (or `main.py`) toggles the LED in the co-simulated blink circuit.
- Missing required peripherals fail the test loudly; they are not ignored.
- The design does not vendor a MicroPython fork in the Ohmforge workspace.

## 7. Open questions

- UART0 versus USB Serial/JTAG as the default REPL on the official C3 image, and which official build to pin.
- Whether `main.py` is copied into a littlefs image we generate, or the CI builds a frozen-manifest image from a tiny in-tree script.
