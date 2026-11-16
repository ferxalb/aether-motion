# CHANGELOG

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/); while the version is `0.x`, the `data-hf-*` attributes and
CLI/MCP output may still change between minor releases.

## [Unreleased]

### Added

- Audio. `<audio src>` clips with `data-start`, `data-duration`, `data-volume`, `data-hf-fade-in`,
  `data-hf-fade-out`, `data-hf-trim-start`, and `data-hf-loop`. Sources can be WAV, FLAC, Ogg Vorbis, or MP3
  (mono or stereo, any sample rate). Clips are mixed onto one 48 kHz stereo timeline with whole-sample
  timing, encoded to AAC-LC with the pure-Rust `oxideav-aac`, and muxed as a second MP4 track whose edit list
  removes the encoder delay, so audio starts exactly on the first video frame.
- `--audio-bitrate` (CLI) and `audioBitrate` (`aether_render`) in kbit/s; `check` reports the audio clips, the
  peak level, and clipped samples; `inspect` lists audio clips.
- `examples/audio.html`.
- `vendor/oxideav-aac`: `oxideav-aac` 0.1.7 with a table-driven forward MDCT (about 5x faster AAC encoding).

### Changed

- `<audio>` is no longer rejected. `--encoder ffmpeg` returns an explicit error for compositions that use it.

## [0.1.0] - 2026-11-15

First stable release.

### Added

- Native renderer for a documented subset of HyperFrames HTML: PNG frames and H.264 MP4 output without a
  browser or Node.js (OpenH264 built in, FFmpeg optional with `--encoder ffmpeg`).
- Layout and paint: flex layout, margins, text wrapping, `border`, `border-radius`, linear and radial
  gradients, `box-shadow`, `text-shadow`, `overflow: hidden`, `transform`, `filter: blur()`,
  `backdrop-filter: blur()`, vignette and film grain.
- Motion: `data-hf-keyframes`, `data-hf-camera`, `data-hf-split`, `data-hf-reveal`, `data-hf-counter`,
  `data-hf-transition`, fades, and CSS `@keyframes` compiled to native tracks.
- Ready-made blocks and themes (`block list`, `block <name>`).
- CLI (`check`, `inspect`, `render`, `bench`, `block`) and an MCP server over stdio (`aether_guide`,
  `aether_write`, `aether_block`, `aether_check`, `aether_snapshot`, `aether_render`).
- 1080p and 4K export presets, multi-threaded rendering with a low-priority default.
- Tile cache, reduced-resolution wide blurs, and identical-frame reuse; see `BENCHMARK.md`.

### Changed

- Updated the stdio MCP server from protocol `2024-11-05` to `2026-07-28`; added `server/discover`, per-request metadata, modern result fields (`resultType`, cache hints, and server identity), and legacy `initialize` compatibility.
- Generic font families (`sans-serif`, `serif`, `monospace`) and the final fallback use an embedded Roboto
  (Apache 2.0), so text renders identically on every machine. Named families still resolve against
  system fonts.
