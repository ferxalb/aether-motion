# CHANGELOG

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/); while the version is `0.x`, the `data-hf-*` attributes and
CLI/MCP output may still change between minor releases.

## [Unreleased]

### Added

- `--quality draft|standard|high|max` (CLI), `"quality"` in `aether.json`, and `quality` in `aether_render`:
  H.264 quality presets with fixed quantizers 30, 26, 22, and 18 (CRF 28, 25, 21, and 18 with
  `--encoder ffmpeg`). `standard` is byte-identical to earlier exports.

### Changed

- MP4 exports default to `--quality max` (quantizer 18). Earlier releases always encoded at OpenH264's
  default quantizer 26: the `24..32` range the encoder was configured with never applied with rate control
  off. Files are larger (about 1.7 times for typical scenes); CPU time is essentially unchanged.
- AAC encoding is about 12 times faster (60 s of stereo audio: 16.7 s down to 1.4 s on one core) with
  byte-identical output: the vendored `oxideav-aac` computes its forward MDCT with an FFT.
- Gradient, glyph, and small translucent fills avoid soft-float `round` and libm `hypot` calls in their
  per-pixel loops, cutting drawing instructions by about 11% on the promo examples with identical pixels.

## [0.1.0] - 2026-11-15

First stable release.

### Added

- npm distribution as `@aether-motion/cli` (launcher plus per-platform binary packages, no install scripts), published from GitHub Actions with trusted publishing; see `docs/releasing.md`. When started through npm, `init` writes an `npx`-based MCP entry if the binary is not on `PATH`.
- `init`, `preview`, `add`, `agent setup|status`, and `doctor` commands; `render` and `check` now work without
  arguments inside a project (`aether.json`). `init` installs the bundled skills and MCP config for coding agents.
- Native renderer for a documented subset of HyperFrames HTML: PNG frames and H.264 MP4 output without a
  browser or Node.js (OpenH264 built in, FFmpeg optional with `--encoder ffmpeg`).
- Layout and paint: flex layout, margins, text wrapping, `border`, `border-radius`, linear and radial
  gradients, `box-shadow`, `text-shadow`, `overflow: hidden`, `transform`, `filter: blur()`,
  `backdrop-filter: blur()`, vignette and film grain.
- Text uses an embedded Roboto (Apache 2.0) for the generic families (`sans-serif`, `serif`,
  `monospace`) and as the final fallback, so text renders identically on every machine. Named families
  still resolve against system fonts.
- Motion: `data-hf-keyframes`, `data-hf-camera`, `data-hf-split`, `data-hf-reveal`, `data-hf-counter`,
  `data-hf-transition`, fades, and CSS `@keyframes` compiled to native tracks.
- Audio. `<audio src>` clips with `data-start`, `data-duration`, `data-volume`, `data-hf-fade-in`,
  `data-hf-fade-out`, `data-hf-trim-start`, and `data-hf-loop`. Sources can be WAV, FLAC, Ogg Vorbis, or MP3
  (mono or stereo, any sample rate). Clips are mixed onto one 48 kHz stereo timeline with whole-sample
  timing, encoded to AAC-LC with the pure-Rust `oxideav-aac`, and muxed as a second MP4 track whose edit list
  removes the encoder delay, so audio starts exactly on the first video frame. Audio needs the native
  encoder; `--encoder ffmpeg` returns an explicit error for compositions that use it.
- `vendor/oxideav-aac`: `oxideav-aac` 0.1.7 with a table-driven forward MDCT (about 5x faster AAC encoding).
- Ready-made blocks and themes (`block list`, `block <name>`).
- CLI (`check`, `inspect`, `render`, `bench`, `block`) and an MCP server over stdio (`aether_guide`,
  `aether_write`, `aether_block`, `aether_check`, `aether_snapshot`, `aether_render`). The server speaks MCP
  protocol `2026-07-28` (with `server/discover`, per-request metadata, and modern result fields) and accepts
  legacy clients that begin with `initialize`.
- `--audio-bitrate` (CLI) and `audioBitrate` (`aether_render`) in kbit/s; `check` reports the audio clips,
  the peak level, and clipped samples; `inspect` lists audio clips.
- 1080p and 4K export presets, multi-threaded rendering with a low-priority default.
- Tile cache, reduced-resolution wide blurs, and identical-frame reuse; see `BENCHMARK.md`.
- `examples/audio.html`.
- Static SVG. Local `<img src="file.svg">` and inline `<svg>` are rasterized deterministically with `resvg`
  at the target size (sharp at any `--scale`; `object-fit`, fades, keyframes, and transitions apply). Shapes,
  paths, strokes, gradients, clip paths, masks, and filters render; `<text>`, SMIL animation,
  `<foreignObject>`, `<script>`, event handlers, and external references are rejected or ignored. `check`
  validates SVG files and inline markup. `examples/svg.html`.
- Clearer errors for unsupported scripts and SVG content: the `fix` hint points to the native `data-hf-*`
  motion and CSS `@keyframes` equivalents.
