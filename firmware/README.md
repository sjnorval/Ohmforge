# Test firmware

Small firmware images that the simulator's tests run. Everything here is our
own code (AGPL-3.0-or-later); third-party binaries are never committed.

| Folder | What it does | Used by |
|---|---|---|
| `esp32c3-blinky` | Bare-metal RV32IMC assembly for the ESP32-C3: prints `ohmforge blinky` on UART0, routes GPIO8 through the IO MUX and GPIO matrix, then toggles it every 250 µs using SYSTIMER deadlines | `crates/of-soc-esp32c3/tests/blinky.rs`, `tests/blinky_led.rs` |
| `esp32c3-timer-irq` | Prints `ohmforge timer irq`, switches off the RTC WDT, both MWDTs and the super watchdog (auto-feed), routes SYSTIMER target 0 (period 250 µs) to CPU interrupt 1 and the TIMG0 timer 0 alarm (1 ms, auto-reload) to CPU interrupt 2 through the interrupt matrix, then sleeps in `wfi`; the vectored handlers toggle GPIO8 and count ticks | `crates/of-soc-esp32c3/tests/timer_irq.rs` |
| `esp32c3-xip-blinky` | A direct-boot flash image (`xip_blinky.bin`, written at flash offset 0; starts with the magic `0xaedb041d` twice) that runs in place from flash: the ROM maps flash into the IROM/DROM windows and jumps to `0x4200_0008`. It switches off the boot watchdogs, prints `ohmforge xip blinky` from DROM, sums a DROM table into `drom_sum`, then toggles GPIO8 every 250 µs (SYSTIMER deadlines) and counts rising edges in `toggles` | `crates/of-soc-esp32c3/tests/rom_boot.rs` (real ROM), `tests/flash_boot.rs` (ROM-lite) |

Each folder holds the source, a linker script, a `build.sh` and the built ELF
(plus the flash image `xip_blinky.bin` for `esp32c3-xip-blinky`). They are
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
```

The build is reproducible: it embeds no paths or build IDs, and `build.sh`
resets the ELF to mode 644. CI rebuilds every image with the pinned zig and
fails if the result differs from the committed file, so commit the source and
the rebuilt ELF (and `.bin`) together.

Espressif's ROM, which boots `esp32c3-xip-blinky` in the ROM tests, is not
firmware of ours and is never committed; `scripts/fetch-esp-rom-elfs.sh`
downloads it (see `AGENTS.md` section 3c).
