#!/usr/bin/env bash
# Download Espressif's ROM ELFs (esp-rom-elfs, Apache-2.0) for the ESP32-C3
# ROM boot tests. The ROM code is Espressif's and not open source, so it is
# never committed: this fetches a pinned release, checks its SHA-256 and
# unpacks it outside target/ (build caches restore target/).
# It also fetches the ESP-IDF second-stage bootloader that espflash ships for
# the ESP32-C3 (Espressif's code, Apache-2.0, built from ESP-IDF; pinned to an
# espflash release and checked by SHA-256), which tests/esp_hal_blinky.rs
# writes at flash offset 0 as espflash does.
#
# Output: $OF_ESP_ROM_DIR, else ${RUNNER_TEMP:-<repo>/.cache}/esp-rom-elfs/<release>
# The tests look in the same places (crates/of-soc-esp32c3/tests/common/mod.rs).
# Needs curl (or wget), tar and sha256sum (or shasum).
# Usage: scripts/fetch-esp-rom-elfs.sh
set -euo pipefail

# Keep in step with of_soc_esp32c3::rom::{ROM_ELFS_RELEASE, ROM_ELFS_TARBALL_SHA256}.
RELEASE=20260528
SHA256=caa463d3cbef2430a5a35847c1d9f2f152403b17a802050927ff60c8da54fe46
ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OF_ESP_ROM_DIR:-${RUNNER_TEMP:-$ROOT/.cache}/esp-rom-elfs/$RELEASE}"
TARBALL="esp-rom-elfs-$RELEASE.tar.gz"
URL="https://github.com/espressif/esp-rom-elfs/releases/download/$RELEASE/$TARBALL"
# Keep in step with of_soc_esp32c3::bootloader::{ESPFLASH_RELEASE, C3_BOOTLOADER_FILE, C3_BOOTLOADER_SHA256}.
ESPFLASH=v4.7.0
BL_FILE="espflash-$ESPFLASH-esp32c3-bootloader.bin"
BL_SHA256=7c04141e1fd8da5a391e8004ffe3d8ee8e20eeb5baf9bd3abc05c03a17b1ed09
BL_URL="https://raw.githubusercontent.com/esp-rs/espflash/$ESPFLASH/espflash/resources/bootloaders/esp32c3-bootloader.bin"

download() { # url file
  if command -v curl >/dev/null; then
    curl -fsSL --retry 3 -o "$2" "$1"
  else
    wget -q -O "$2" "$1"
  fi
}
check() { # file sha256
  local got
  if command -v sha256sum >/dev/null; then
    got="$(sha256sum "$1" | cut -d' ' -f1)"
  else
    got="$(shasum -a 256 "$1" | cut -d' ' -f1)"
  fi
  if [ "$got" != "$2" ]; then
    echo "error: $(basename "$1") has SHA-256 $got, expected $2" >&2; exit 1
  fi
}

mkdir -p "$OUT"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
if [ -f "$OUT/esp32c3_rev3_rom.elf" ]; then
  echo "esp-rom-elfs $RELEASE already in $OUT"
else
  download "$URL" "$tmp/$TARBALL"
  check "$tmp/$TARBALL" "$SHA256"
  tar -xzf "$tmp/$TARBALL" -C "$OUT"
  echo "unpacked esp-rom-elfs $RELEASE into $OUT"
fi
if [ -f "$OUT/$BL_FILE" ]; then
  echo "$BL_FILE already in $OUT"
else
  download "$BL_URL" "$tmp/$BL_FILE"
  check "$tmp/$BL_FILE" "$BL_SHA256"
  mv "$tmp/$BL_FILE" "$OUT/$BL_FILE"
  echo "fetched $BL_FILE into $OUT"
fi
echo "run: cargo test -p of-soc-esp32c3 --test rom_boot --test esp_hal_blinky"
