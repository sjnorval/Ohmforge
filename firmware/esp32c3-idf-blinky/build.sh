#!/usr/bin/env bash
# Build the ESP-IDF blinky and its flash image.
#
# Needs ESP-IDF v5.3.2 with the ESP32-C3 target installed (`idf.py` on PATH
# after `. $IDF_PATH/export.sh`), or Docker with the pinned image:
#   docker run --rm -v "$PWD/../..:/project" -w /project/firmware/esp32c3-idf-blinky \
#     -e SOURCE_DATE_EPOCH=1767225600 espressif/idf:v5.3.2 ./build.sh
#
# Outputs (committed, mode 644):
#   partitions.bin   partition table, flash 0x8000
#   app.bin          the ESP-IDF app image, flash 0x10000
#
# The second-stage bootloader is Espressif's (fetched, never committed);
# tests use espflash's pinned bootloader like the esp-hal demos.
set -euo pipefail
cd "$(dirname "$0")"

want_idf="v5.3.2"
if [ -z "${IDF_PATH:-}" ]; then
  echo "error: IDF_PATH is unset; source ESP-IDF $want_idf export.sh first" >&2
  exit 1
fi
# IDF_VER is set by export.sh (e.g. v5.3.2).
have_idf="${IDF_VER:-unknown}"
case "$have_idf" in
  "$want_idf"|"$want_idf"*) ;;
  *)
    # Docker images and some installs report the version via idf.py.
    reported="$(idf.py --version 2>/dev/null || true)"
    case "$reported" in
      *"$want_idf"*) ;;
      *)
        echo "error: need ESP-IDF $want_idf, found IDF_VER=$have_idf idf.py=$reported" >&2
        exit 1
        ;;
    esac
    ;;
esac

# App descriptor / build metadata: pin the date so rebuilds match.
export SOURCE_DATE_EPOCH="${SOURCE_DATE_EPOCH:-1767225600}"

idf.py set-target esp32c3
idf.py build

part=build/partition_table/partition-table.bin
app=build/esp32c3-idf-blinky.bin
[ -f "$part" ] || { echo "error: missing $part" >&2; exit 1; }
[ -f "$app" ] || { echo "error: missing $app" >&2; exit 1; }

cp "$part" partitions.bin
cp "$app" app.bin
chmod 644 partitions.bin app.bin
echo "wrote partitions.bin ($(wc -c < partitions.bin) bytes) and app.bin ($(wc -c < app.bin) bytes)"
