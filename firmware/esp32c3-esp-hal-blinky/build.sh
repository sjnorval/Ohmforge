#!/usr/bin/env bash
# Build the esp-hal blinky (Rust, esp-hal) and its flash image.
#
# Needs: Rust 1.95.0 with the riscv32imc-unknown-none-elf target (rustup
# installs both from rust-toolchain.toml) and espflash 4.7.0 (`cargo install
# espflash@4.7.0 --locked`, or a release binary; set ESPFLASH to its path).
#
# Outputs (committed, mode 644):
#   esp_hal_blinky.elf  the app ELF (symbols, no debug info)
#   partitions.bin      espflash's default partition table, flash 0x8000
#   app.bin             the ESP-IDF-format app image, flash 0x10000
# They are cut from `espflash save-image --merge` for a 4 MB flash. The
# merged image also starts with espflash's ESP-IDF second-stage bootloader
# at 0x0, which is Espressif's (Apache-2.0) and is not committed: the tests
# fetch it (scripts/fetch-esp-rom-elfs.sh) and patch its header the way
# espflash does. Set OF_CHECK_BOOTLOADER to that bootloader file to check
# that the merged image really is that bootloader plus these two files.
set -euo pipefail
cd "$(dirname "$0")"
ESPFLASH="${ESPFLASH:-espflash}"
want="espflash 4.7.0"
have="$("$ESPFLASH" --version)"
[ "$have" = "$want" ] || { echo "error: need $want, found $have" >&2; exit 1; }

# The ESP-IDF app descriptor records a build date and time: pin them
# (2026-01-01 00:00:00 UTC) so rebuilds are byte-identical.
export SOURCE_DATE_EPOCH=1767225600
cargo_home="${CARGO_HOME:-$HOME/.cargo}"
# Arrays from --config are added to .cargo/config.toml's rustflags.
remap="target.riscv32imc-unknown-none-elf.rustflags=[\"--remap-path-prefix=$cargo_home=/cargo\", \"--remap-path-prefix=$PWD=/fw\"]"
cargo build --release --locked --config "$remap"
elf=target/riscv32imc-unknown-none-elf/release/esp32c3-esp-hal-blinky

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
"$ESPFLASH" save-image --chip esp32c3 --merge --flash-size 4mb --skip-padding "$elf" "$tmp/merged.bin" >/dev/null
cp "$elf" esp_hal_blinky.elf
# Partition table: one 3 KiB sector at 0x8000. App: everything from 0x10000.
dd if="$tmp/merged.bin" of=partitions.bin bs=1024 skip=32 count=3 status=none
dd if="$tmp/merged.bin" of=app.bin bs=1024 skip=64 status=none
chmod 644 esp_hal_blinky.elf partitions.bin app.bin

if [ -n "${OF_CHECK_BOOTLOADER:-}" ]; then
  # espflash writes the flash size and frequency into header byte 3 (0x20:
  # 4 MB, 40 MHz) and re-appends the image SHA-256; the rest of the
  # bootloader is unchanged.
  python3 - "$OF_CHECK_BOOTLOADER" "$tmp/merged.bin" <<'PY'
import hashlib, sys
bl = bytearray(open(sys.argv[1], 'rb').read())
merged = open(sys.argv[2], 'rb').read()
bl[3] = 0x20
bl[-32:] = hashlib.sha256(bl[:-32]).digest()
assert merged[:len(bl)] == bl, "bootloader differs from the fetched one"
assert all(b == 0xff for b in merged[len(bl):0x8000]), "unexpected data before 0x8000"
assert merged[0x8000:0x8c00] == open('partitions.bin', 'rb').read()
assert all(b == 0xff for b in merged[0x8c00:0x10000])
assert merged[0x10000:] == open('app.bin', 'rb').read()
print("merged image = fetched bootloader + partitions.bin + app.bin")
PY
fi
