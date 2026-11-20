#!/usr/bin/env bash
# Rebuild xip_blinky.elf and the flash image xip_blinky.bin (written at
# flash offset 0; direct boot). Needs zig (`pip install ziglang`), which
# bundles clang, lld and objcopy for riscv32. Both files are committed so
# tests need no toolchain; CI rebuilds them and checks they are
# byte-identical.
set -euo pipefail
cd "$(dirname "$0")"
ZIG=${ZIG:-python3 -m ziglang}
$ZIG cc -target riscv32-freestanding-none -mcpu=generic_rv32+m+c \
    -nostdlib -static -g0 -Wl,-T,link.ld -Wl,--build-id=none -Wl,-z,norelro -Wl,-z,max-page-size=16 \
    -o xip_blinky.elf xip_blinky.S
# The flash image is the loadable bytes from 0x42000000 on (code, then
# read-only data at the matching flash offset).
$ZIG objcopy -O binary xip_blinky.elf xip_blinky.bin
# Linkers mark their output executable; the committed files are data (100644).
chmod 644 xip_blinky.elf xip_blinky.bin
echo "built $(wc -c < xip_blinky.elf) + $(wc -c < xip_blinky.bin) bytes"
