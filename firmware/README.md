# Test firmware

Small firmware images that the simulator's tests run. Everything here is our
own code (AGPL-3.0-or-later; the esp-hal blinky links permissively licensed
Rust crates, see below); third-party binaries are never committed.

| Folder | What it does | Used by |
|---|---|---|
| `esp32c3-blinky` | Bare-metal RV32IMC assembly for the ESP32-C3: prints `ohmforge blinky` on UART0, routes GPIO8 through the IO MUX and GPIO matrix, then toggles it every 250 µs using SYSTIMER deadlines | `crates/of-soc-esp32c3/tests/blinky.rs`, `tests/blinky_led.rs` |
| `esp32c3-timer-irq` | Prints `ohmforge timer irq`, switches off the RTC WDT, both MWDTs and the super watchdog (auto-feed), routes SYSTIMER target 0 (period 250 µs) to CPU interrupt 1 and the TIMG0 timer 0 alarm (1 ms, auto-reload) to CPU interrupt 2 through the interrupt matrix, then sleeps in `wfi`; the vectored handlers toggle GPIO8 and count ticks | `crates/of-soc-esp32c3/tests/timer_irq.rs` |
| `esp32c3-clock-counter` | Counts a 555 clock on GPIO4: waits 2 ms (SYSTIMER) for the rail, prints `ohmforge clock counter` on UART0, switches off the boot watchdogs, enables the GPIO4 input (IO MUX `FUN_IE`) with a rising-edge interrupt routed through the interrupt matrix to CPU interrupt 1, then sleeps in `wfi`. The handler timestamps each edge with SYSTIMER and closes a 100 ms gate on a whole period; the main loop prints `freq: <Hz with two decimals> Hz (<n> periods)`, stores it in `freq_chz` and toggles GPIO8 once per gate | `circuits/demos/esp32c3-555-clock.circuit.toml` (direct boot, no ROM): `crates/of-ui` session test and the `of-circuit` demo tests |
| `esp32c3-xip-blinky` | A direct-boot flash image (`xip_blinky.bin`, written at flash offset 0; starts with the magic `0xaedb041d` twice) that runs in place from flash: the ROM maps flash into the IROM/DROM windows and jumps to `0x4200_0008`. It switches off the boot watchdogs, prints `ohmforge xip blinky` from DROM, sums a DROM table into `drom_sum`, then toggles GPIO8 every 250 µs (SYSTIMER deadlines) and counts rising edges in `toggles` | `crates/of-soc-esp32c3/tests/rom_boot.rs` (real ROM), `tests/flash_boot.rs` (ROM-lite) |
| `esp32c3-esp-hal-blinky` | An unmodified esp-hal 1.2.2 app in Rust: `esp_hal::init` with `CpuClock::max()` (160 MHz), prints `ohmforge esp-hal blinky` on UART0, then toggles GPIO8 every 500 µs (`Instant` deadlines). Built as espflash builds it: `partitions.bin` (flash 0x8000) and `app.bin` (ESP-IDF app image with its app descriptor, flash 0x10000) are cut from `espflash save-image --merge`; the real ROM boots it through espflash's ESP-IDF bootloader | `crates/of-soc-esp32c3/tests/esp_hal_blinky.rs` (real ROM and bootloader) |
| `esp32c3-esp-hal-button` | Two esp-hal 1.2.2 apps for a push button on GPIO9 (internal pull-up, pressed = low) and an LED on GPIO8, with no debouncing. `button_poll.bin` copies GPIO9 to the LED in a tight loop and prints `poll: <n> <level>` on each change; `button_irq.bin` toggles the LED from a GPIO9 falling-edge interrupt (esp-hal `Io` handler), sleeps in `wfi` and prints `irq: <n>`. Built like the blinky (`partitions.bin` plus one app image each, flash 0x10000) | `crates/of-soc-esp32c3/tests/esp_hal_button.rs` (real ROM and bootloader, analog push button) |
| `esp32c3-esp-hal-adc-pwm` | Two esp-hal 1.2.2 apps for a potentiometer wiper on GPIO2 (ADC1 channel 2, 11 dB) and an LED on GPIO8 (LEDC channel 0, timer 0, APB clock). `pot_dimmer.bin` reads the pot every 5 ms with esp-hal's curve calibration (`AdcCalCurve`), sets a 1 kHz, 10-bit duty (0 to 2500 mV maps to 0 to 100 %) and prints `adc: <mV> duty: <duty>`; `led_fade.bin` fades the LED from 0 % to 100 % and back with the LEDC hardware fade (5 kHz, 8-bit, 255 steps of one period each) and prints `fade: <n> up` / `fade: <n> down`. Built like the blinky (`partitions.bin` plus one app image each, flash 0x10000) | `crates/of-soc-esp32c3/tests/esp_hal_adc_pwm.rs` (real ROM and bootloader, potentiometer and LED circuit) |
| `esp32c3-idf-blinky` | ESP-IDF v5.3.2 app: prints `ohmforge idf blinky` on UART0, toggles GPIO8 every 500 µs (`esp_rom_delay_us`). Built with `idf.py` (or `espressif/idf:v5.3.2` Docker); committed `partitions.bin` + `app.bin` | `crates/of-soc-esp32c3/tests/idf_blinky.rs` (real ROM and bootloader) |
| Arduino-ESP32 / remaining ESP-IDF MVP apps | Still planned (#60): button, dimmer, fade, and Arduino-ESP32 counterparts. Design: [`docs/design/arduino-idf-mvp-demos.md`](../docs/design/arduino-idf-mvp-demos.md). | — |

The assembly folders each hold the source, a linker script, a `build.sh` and
the built ELF (plus the flash image `xip_blinky.bin` for `esp32c3-xip-blinky`). They are
committed (a few KB) so the tests need no RISC-V toolchain.

`esp32c3-blinky` and `esp32c3-timer-irq` keep `.rodata` at `0x3c01_0020`, not
`0x3c00_0020`: the IROM and DROM windows share one cache MMU table, so code at
`0x4200_0020` and data at the same window offset would need the same flash
bytes.

## Rebuilding

The build needs zig, which bundles clang and lld for riscv32:

```sh
pip install ziglang==0.17.1
bash firmware/esp32c3-blinky/build.sh
bash firmware/esp32c3-timer-irq/build.sh
bash firmware/esp32c3-xip-blinky/build.sh
bash firmware/esp32c3-clock-counter/build.sh
```

The build is reproducible: it embeds no paths or build IDs, and `build.sh`
resets the ELF to mode 644. CI rebuilds every image with the pinned zig and
fails if the result differs from the committed file, so commit the source and
the rebuilt ELF (and `.bin`) together.

Espressif's ROM, which boots `esp32c3-xip-blinky` in the ROM tests, is not
firmware of ours and is never committed; `scripts/fetch-esp-rom-elfs.sh`
downloads it (see `AGENTS.md` section 3c).

## esp32c3-esp-hal-blinky, esp32c3-esp-hal-button and esp32c3-esp-hal-adc-pwm

`esp32c3-esp-hal-button` is built the same way (`build.sh` there; it adds
esp-hal's `unstable` feature for the GPIO interrupt API and the
`critical-section` crate, MIT OR Apache-2.0) and commits only
`partitions.bin`, `button_poll.bin` and `button_irq.bin` (the ELFs stay in
`target/`). `esp32c3-esp-hal-adc-pwm` is built the same way too (esp-hal's
`unstable` feature for the ADC and LEDC drivers) and commits
`partitions.bin`, `pot_dimmer.bin` and `led_fade.bin`. The rest of this
section applies to all three.

A Rust crate with its own Cargo workspace, `Cargo.lock` and pinned toolchain
(Rust 1.95.0, `riscv32imc-unknown-none-elf`, from `rust-toolchain.toml`). It
is built with cargo and espflash 4.7.0, not zig:

```sh
cargo install espflash@4.7.0 --locked   # or a release binary, then ESPFLASH=/path/to/espflash
bash firmware/esp32c3-esp-hal-blinky/build.sh
```

The build is reproducible: `build.sh` remaps the cargo home and source paths,
pins `SOURCE_DATE_EPOCH` (the app descriptor's build date and time read
2026-01-01 00:00:00) and resets the outputs to mode 644. CI rebuilds it with
the pinned espflash release binary and fails on any difference.

The merged image espflash writes also holds Espressif's ESP-IDF second-stage
bootloader at flash 0 (espflash's prebuilt `esp32c3-bootloader.bin`, built
from ESP-IDF, Apache-2.0). It is not ours and is not committed:
`scripts/fetch-esp-rom-elfs.sh` fetches the pinned file and the test writes
it as espflash does (`of_soc_esp32c3::bootloader::espflash_flash_image`).
`OF_CHECK_BOOTLOADER=<fetched file> build.sh` checks that espflash's merged
image is exactly that bootloader plus the two committed files.

Licences: the app is AGPL-3.0-or-later; the crates it links are permissive
(esp-hal, esp-bootloader-esp-idf, esp-println, esp-riscv-rt and most of the
rest MIT OR Apache-2.0; riscv and riscv-rt ISC; a few MIT), so the committed
`app.bin` and ELF carry their code under those licences (`Cargo.lock` lists
them). espflash is MIT OR Apache-2.0.
