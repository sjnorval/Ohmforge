#!/usr/bin/env bash
# Rebuild clock.elf. Needs zig (`pip install ziglang`), which bundles
# clang and lld for riscv32. Built like esp32c3-blinky: reproducible (no
# paths or build IDs), mode 644.
set -euo pipefail
cd "$(dirname "$0")"
ZIGCC=${ZIGCC:-python3 -m ziglang cc}
$ZIGCC -target riscv32-freestanding-none -mcpu=generic_rv32+m+c \
    -nostdlib -static -g0 -Wl,-T,link.ld -Wl,--build-id=none -Wl,-z,norelro -Wl,-z,max-page-size=16 \
    -o clock.elf clock.S
chmod 644 clock.elf
echo "built $(wc -c < clock.elf) bytes"
