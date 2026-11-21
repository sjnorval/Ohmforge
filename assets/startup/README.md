# Ohmforge startup animation

Assets for the Ohmforge desktop load screen, in a warm neon “digital fire forge” style (cyberpunk neon and glitch, Matrix-style digital rain in amber/orange, and Tesla-like industrial geometry).

Licence: [CC-BY-SA-4.0](../../parts/LICENSE.md), same as other Ohmforge artwork, parts data and docs.

## Committed files

- [`EILIi.mp4`](EILIi.mp4) — the source animation: 6.04 s, 24 fps, 736×400, social-preview framing (text left, board right). Concept name: EILIi. The audio track is not used.
- [`startup-still.png`](startup-still.png) — the last frame of `EILIi.mp4`, same 736×400 size (poster / fallback still).
- [`splash.ofsp`](splash.ofsp) — packed JPEG stills (145 frames, 24 fps, 736×400, about 6.1 MiB) played by the desktop app as its load screen.

These replace an earlier broken `startup-still.jpg` placeholder (103 bytes) that was not a usable image.

## Load screen

The desktop app (`cargo run -p of-app --features gui`) plays `splash.ofsp` in real time over about 6 s while the demo session starts, then fades into the main window in 300 ms. Click, Escape, or any other key skips to the last frame and, once the demo is ready, starts the fade. If initialisation is still running, the last frame is held until it finishes.

Disable the splash with `--no-splash` or by setting `OF_NO_SPLASH` to `1`, `true`, `yes`, or `on`. There is no persistent settings toggle yet.

The app does not ship a video decoder (no ffmpeg or GStreamer is linked). Audio is dropped. The WASM crate (`of-web`) is still a placeholder and is not given these frames; the wasm CI target set is unchanged.

## Regenerating the pack

`scripts/gen-startup-frames.sh` (or `scripts/gen-startup-frames.ps1`) uses ffmpeg as an **external** tool to extract native-resolution 24 fps JPEGs (`-q:v 8`) and pack them into `splash.ofsp`. ffmpeg must be on `PATH`. The pack format is little-endian `OFSP` version 1: `width`, `height`, `fps_num`, `fps_den`, `frame_count`, then each frame as a `u32` length and a JPEG or PNG payload.

An animated WebP of this clip was larger than the source MP4 (the rain and glitch change most of the frame every tick), so packed JPEG stills are used instead. Decode uses the `image` crate (MIT OR Apache-2.0) already in the tree, with its `jpeg` and `png` features.

## Other concepts

Three other concept videos (`concept-1.mp4`, `concept-2.mp4`, `concept-4.mp4`) are not committed, to keep the repository lean. They were attached to the pull request that landed the source animation.
