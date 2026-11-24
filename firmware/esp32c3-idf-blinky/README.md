# esp32c3-idf-blinky

Unmodified ESP-IDF v5.3.2 blinky for the ESP32-C3 (AGPL-3.0-or-later).

Prints `ohmforge idf blinky` on UART0 and toggles GPIO8 every 500 µs. The
simulator boots it through the real ROM and espflash’s ESP-IDF second-stage
bootloader (same path as the esp-hal demos).

## Build

Needs ESP-IDF **v5.3.2** (`idf.py` after `export.sh`), or Docker:

```bash
docker run --rm -v "$PWD/../..:/project" -w /project/firmware/esp32c3-idf-blinky \
  -e SOURCE_DATE_EPOCH=1767225600 espressif/idf:v5.3.2 ./build.sh
```

Outputs (committed, mode 644): `partitions.bin` (flash `0x8000`), `app.bin`
(flash `0x10000`). Do not commit `build/` or `sdkconfig`.

## Tests

`crates/of-soc-esp32c3/tests/idf_blinky.rs` (needs the fetched ROM/bootloader).
