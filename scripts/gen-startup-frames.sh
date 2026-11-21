#!/usr/bin/env bash
# Convert assets/startup/EILIi.mp4 into the packed splash frames the desktop
# app plays as its load screen. ffmpeg is used only as an external tool; the
# app never links a video decoder.
#
# Output: assets/startup/splash.ofsp (see assets/startup/README.md).
# Needs ffmpeg (and ffprobe, optional) on PATH.
# Usage: scripts/gen-startup-frames.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SRC="$ROOT/assets/startup/EILIi.mp4"
OUT="$ROOT/assets/startup/splash.ofsp"
# Native 24 fps, 736x400. -q:v 8 is a size/quality trade-off for this
# high-motion clip; animated WebP was larger than the source MP4.
WIDTH=736
HEIGHT=400
FPS_NUM=24
FPS_DEN=1
QV=8

if ! command -v ffmpeg >/dev/null; then
  echo "ffmpeg not found on PATH; install it to regenerate $OUT" >&2
  exit 1
fi
if [[ ! -f "$SRC" ]]; then
  echo "missing source video: $SRC" >&2
  exit 1
fi

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# -map 0:v:0 skips the MP4's attached thumbnail stream.
ffmpeg -y -hide_banner -loglevel error \
  -i "$SRC" -map 0:v:0 -an -q:v "$QV" -start_number 0 \
  "$TMP/frame-%03d.jpg"

python3 - "$TMP" "$OUT" "$WIDTH" "$HEIGHT" "$FPS_NUM" "$FPS_DEN" <<'PY'
import pathlib, struct, sys

tmp, out, width, height, fps_num, fps_den = sys.argv[1:]
frames = sorted(pathlib.Path(tmp).glob("frame-*.jpg"))
if not frames:
    raise SystemExit("no JPEG frames written")
outp = pathlib.Path(out)
with outp.open("wb") as f:
    f.write(b"OFSP")
    f.write(struct.pack(
        "<IIIIII",
        1,
        int(width),
        int(height),
        int(fps_num),
        int(fps_den),
        len(frames),
    ))
    for path in frames:
        data = path.read_bytes()
        if not data.startswith(b"\xff\xd8"):
            raise SystemExit(f"not a JPEG: {path}")
        f.write(struct.pack("<I", len(data)))
        f.write(data)
print(f"wrote {outp} ({len(frames)} frames, {outp.stat().st_size} bytes)")
PY
