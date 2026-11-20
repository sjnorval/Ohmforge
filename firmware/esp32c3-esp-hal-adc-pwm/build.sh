#!/usr/bin/env bash
# Build the esp-hal ADC and PWM firmware (two apps: pot dimmer and LED fade)
# and their flash images.
#
# Needs: Rust 1.95.0 with the riscv32imc-unknown-none-elf target (rustup
# installs both from rust-toolchain.toml) and espflash 4.7.0 (`cargo install
# espflash@4.7.0 --locked`, or a release binary; set ESPFLASH to its path).
#
# Outputs (committed, mode 644):
#   partitions.bin    espflash's default partition table, flash 0x8000
#   pot_dimmer.bin    the pot dimmer app (ESP-IDF app image), flash 0x10000
#   led_fade.bin      the LED fade app (ESP-IDF app image), flash 0x10000
# They are cut from `espflash save-image --merge` for a 4 MB flash; the ELFs
# stay in target/. The merged image also starts with espflash's ESP-IDF
# second-stage bootloader, which is Espressif's (Apache-2.0) and is not
# committed. Set OF_CHECK_BOOTLOADER to the fetched bootloader
# (scripts/fetch-esp-rom-elfs.sh) to check that each merged image really is
# that bootloader plus these files.
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

tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
for app in pot-dimmer led-fade; do
  elf=target/riscv32imc-unknown-none-elf/release/$app
  "$ESPFLASH" save-image --chip esp32c3 --merge --flash-size 4mb --skip-padding "$elf" "$tmp/$app.bin" >/dev/null
  # Partition table: one 3 KiB sector at 0x8000. App: everything from 0x10000.
  dd if="$tmp/$app.bin" of="$tmp/$app.partitions.bin" bs=1024 skip=32 count=3 status=none
  dd if="$tmp/$app.bin" of="${app//-/_}.bin" bs=1024 skip=64 status=none
done
cmp "$tmp/pot-dimmer.partitions.bin" "$tmp/led-fade.partitions.bin"
cp "$tmp/pot-dimmer.partitions.bin" partitions.bin
chmod 644 partitions.bin pot_dimmer.bin led_fade.bin

if [ -n "${OF_CHECK_BOOTLOADER:-}" ]; then
  # espflash writes the flash size and frequency into header byte 3 (0x20:
  # 4 MB, 40 MHz) and re-appends the image SHA-256; the rest of the
  # bootloader is unchanged.
  for app in pot-dimmer led-fade; do
    python3 - "$OF_CHECK_BOOTLOADER" "$tmp/$app.bin" "${app//-/_}.bin" <<'PY'
import hashlib, sys
bl = bytearray(open(sys.argv[1], 'rb').read())
merged = open(sys.argv[2], 'rb').read()
bl[3] = 0x20
bl[-32:] = hashlib.sha256(bl[:-32]).digest()
assert merged[:len(bl)] == bl, "bootloader differs from the fetched one"
assert all(b == 0xff for b in merged[len(bl):0x8000]), "unexpected data before 0x8000"
assert merged[0x8000:0x8c00] == open('partitions.bin', 'rb').read()
assert all(b == 0xff for b in merged[0x8c00:0x10000])
assert merged[0x10000:] == open(sys.argv[3], 'rb').read()
print(f"merged image = fetched bootloader + partitions.bin + {sys.argv[3]}")
PY
  done
fi
