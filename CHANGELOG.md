# Changelog

All notable changes to Ohmforge are recorded here. The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/), and the project will use [Semantic Versioning](https://semver.org/) from its first release (0.1, the end of P2; see [`docs/ROADMAP.md`](docs/ROADMAP.md)).

Add a line under **Unreleased** in every pull request with a user-visible change, in the matching section (Added, Changed, Deprecated, Removed, Fixed, Security). GitHub's generated release notes (`.github/release.yml`) list the pull requests; this file is the curated summary.

## [Unreleased]

Nothing has been released yet. Everything below is on `main` and will ship in 0.1.

### Added

- **esp-hal blinky through the full boot path**: an unmodified esp-hal 1.2.2 blinky (`firmware/esp32c3-esp-hal-blinky`, built with espflash 4.7.0, reproducible) boots on the ESP32-C3 model through the real ROM and espflash's ESP-IDF second-stage bootloader (fetched with the ROM, never committed) and drives the LED circuit. New models: the CPU clock (SYSTEM), the RNG, PMP and debug trigger CSRs, TIMG RTC calibration, UART0 configuration registers and GPIO pin and input select registers.
- **ESP32-C3 ROM boot, flash and cache MMU:** a 4 MiB SPI flash chip behind the SPI0/SPI1 memory controllers and the cache MMU (64 KiB pages into the IROM and DROM windows); Espressif's real mask ROM runs on the model (`Esp32c3::power_on_rom`): it prints its banner and reset cause, direct boots flash, loads and SHA-256-checks an image at flash offset 0, reboots after watchdog resets and honours download-mode strapping. The ROM ELF comes from esp-rom-elfs and is never committed: `scripts/fetch-esp-rom-elfs.sh`/`.ps1` fetch a pinned, checksummed release, and the ROM tests skip without it. A ROM-lite fast boot (`Esp32c3::boot_flash`) boots direct-boot flash, the app from the partition table at 0x8000, or the image at 0, without running the ROM. Also new: partition table parsing and writing (with the MD5 row), 64 KiB-aligned flash image layout (`EspImage::align_flash_segments`), the SHA accelerator, eFuse (blank chip), the USB Serial/JTAG console, the C3 performance-counter CSRs, and an XIP test firmware (`firmware/esp32c3-xip-blinky`) that runs in place from flash.
- **Community and contributor setup:** issue forms (bug report, feature request, new part or model), a pull request checklist, `SECURITY.md` with private vulnerability reporting, `SUPPORT.md`, Dependabot for Cargo and GitHub Actions, this changelog and release-note categories.
- **Desktop blink demo** (#11): `cargo run -p of-app --features gui` shows a live ESP32-C3 blink schematic, GPIO8 through a 330 Ω resistor to a red LED, with the LED lit from the simulated current, an LED current trace and the firmware's UART0 output. The desktop app now uses eframe's wgpu backend.
- **ESP32-C3 interrupts, timers and watchdogs** (#10): the interrupt matrix and CPU interrupt controller (62 sources, 31 lines, priority, threshold, edge and level), traps into vectored `mtvec`, SYSTIMER alarms in target and period mode, timer groups TIMG0/1, and the MWDT, RTC and super watchdogs, whose resets reboot the loaded image. New timer-interrupt test firmware in `firmware/esp32c3-timer-irq`.
- **ESP flash images** (#10): a parser and writer for the ESP image format (header, segments, checksum, optional SHA-256, with our own SHA-256), and boot from a parsed image with a chip ID check.
- **ESP32-C3 blinky end to end** (#8): an ELF32 RISC-V reader (`of-firmware`) and direct ELF boot; the C3 memory map, GPIO and IO MUX output with a Thevenin pin model, UART0 TX capture and SYSTIMER; `Esp32c3Engine` under the `of-core` kernel; and a zig-built blinky firmware that drives a parts-library LED and resistor in the analog solver, with CI checking the firmware rebuilds byte for byte.
- **RISC-V CPU core** (#7): an RV32IMC (+A optional) interpreter with Zicsr, machine-mode traps and interrupts, a cycle model and a debugger API, passing 75 of 77 riscv-tests (2 expected-unsupported).
- **Analog core** (#7): a sparse MNA solver (faer sparse LU) with DC operating point, nested DC sweeps, AC and transient analyses; R, C, L, K, independent and controlled sources (including POLY), switches, diodes and LEDs, Gummel-Poon BJTs and level 1 and 3 MOSFETs, with temperature effects; a SPICE netlist subset parser; `ohmforge run netlist.cir` with CSV and VCD output; an AMS1117-3.3 macromodel; the parts-to-solver bridge; and the analog engine under the `of-core` kernel. 39 reference circuits match ngspice within 1 %.
- **Foundations:** the Cargo workspace, the `of-core` deterministic kernel (`Engine` trait, quantum slices, breakpoints), the `part.toml` loader and example parts, CI on Linux, Windows and macOS with fmt, clippy, cargo-deny, wasm32 and GUI checks, the README banner, favicon and app icon, and UX research specs (#1, #3, #5).

### Changed

- The ESP32-C3 core performs misaligned loads and stores in hardware, as the chip does, and its performance counter counts from reset; the hart follows the CPU clock the firmware selects instead of always running at 160 MHz.
- ESP32-C3 flash windows are read through the cache MMU, which IROM and DROM share; the test firmware's read-only data moved to 0x3c01_0020 so it does not share flash with code.
- The CPU core's `mie`/`mip` CSRs are optional (`HartConfig::mie_mip`); they are off for the ESP32-C3, which has neither (#10).
- Every change to `main` needs the maintainer's review through `CODEOWNERS` and the main-branch ruleset (#9).
- The README has a roadmap summary, and the roadmap has a parked "Future ideas" section (#12).
- Relicensed: code is AGPL-3.0-or-later and parts data, artwork and docs are CC-BY-SA-4.0 (previously MIT OR Apache-2.0 and CC-BY-4.0) (#6).

### Fixed

- Clippy warnings in `of-analog` (#2), and riscv-tests CI runs with a stale cache (#7).

[Unreleased]: https://github.com/sjnorval/Ohmforge/commits/main
