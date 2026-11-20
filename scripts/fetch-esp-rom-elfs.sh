#!/usr/bin/env bash
# Download Espressif's ROM ELFs (esp-rom-elfs, Apache-2.0) for the ESP32-C3
# ROM boot tests. The ROM code is Espressif's and not open source, so it is
# never committed: this fetches a pinned release, checks its SHA-256 and
# unpacks it outside target/ (build caches restore target/).
#
# Output: $OF_ESP_ROM_DIR, else ${RUNNER_TEMP:-<repo>/.cache}/esp-rom-elfs/<release>
# The tests look in the same places (crates/of-soc-esp32c3/tests/rom_boot.rs).
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

if [ -f "$OUT/esp32c3_rev3_rom.elf" ]; then
  echo "esp-rom-elfs $RELEASE already in $OUT"; exit 0
fi
mkdir -p "$OUT"
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
if command -v curl >/dev/null; then
  curl -fsSL --retry 3 -o "$tmp/$TARBALL" "$URL"
else
  wget -q -O "$tmp/$TARBALL" "$URL"
fi
if command -v sha256sum >/dev/null; then
  got="$(sha256sum "$tmp/$TARBALL" | cut -d' ' -f1)"
else
  got="$(shasum -a 256 "$tmp/$TARBALL" | cut -d' ' -f1)"
fi
if [ "$got" != "$SHA256" ]; then
  echo "error: $TARBALL has SHA-256 $got, expected $SHA256" >&2; exit 1
fi
tar -xzf "$tmp/$TARBALL" -C "$OUT"
echo "unpacked esp-rom-elfs $RELEASE into $OUT"
echo "run: cargo test -p of-soc-esp32c3 --test rom_boot"
