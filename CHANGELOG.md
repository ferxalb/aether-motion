# CHANGELOG

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/); while the version is `0.x`, the `data-hf-*` attributes and
CLI/MCP output may still change between minor releases.

## [Unreleased]

### Added

- Inline rich text: `<span>`, `<strong>`, `<b>`, and `<code>` inside an element with its own text become
  styled runs (color, family, weight, letter spacing, fill, stroke, gradient) that wrap and align together.
  `-webkit-text-stroke` (stroked from the font outlines), `-webkit-text-fill-color`, and
  `background-clip: text` gradient text. Plain text renders byte-identically. See `examples/rich-text.html`.
- `data-hf-draw` on inline `<svg>`: strokes draw themselves along their measured length, staggered in
  document order. `data-hf-motion-path`: any element travels along SVG path data (arcs included), with
  optional `-rotate="auto"`. A new `vector` module measures paths. See `examples/vector-motion.html`.
- `clip-path` with `inset()`, `circle()`, `ellipse()`, and `polygon()` (px or %), drawn with exact
  antialiased coverage and animated through `clipPath` in `data-hf-keyframes`; `mask-image` with a linear
  or radial gradient. Both apply after the element's `filter`. See `examples/clip-mask.html`.
- Scene transitions `dip-to-black`, `dip-to-white`, `flash`, `blur`, `whip-left|right|up|down`, `glitch`,
  and `rgb-split`, compiled to filter tracks and new pixel-effect tracks drawn through the effect layer.
  `data-hf-keyframes` accepts `glitch` (0 to 1, a deterministic slice displacement that changes 24 times a
  second) and `rgbSplit` (px). See `examples/light-transitions.html`.
- Audio effects on `<audio>` and the sound of `<video>`: `data-hf-volume-ramp` (eased gain keyframes in
  `volume` or `db`), `data-hf-duck` (sidechain ducking under trigger clips with look-ahead attack, hold,
  release, amount, and threshold), and `data-hf-eq` (up to 16 RBJ biquad bands or the `voice`, `music-bed`,
  `telephone`, `warm`, and `bright` presets). `check`/`aether_check` report `audio.clips[].effects` with the
  EQ response, ducked regions, and automation keys. Clips without effects mix byte-identically. See
  `examples/audio-effects.html`.
- Variable speed on `<video>` and `<audio>`: `data-hf-speed` (constant rate, 0.05 to 16) and
  `data-hf-speed-ramp` (JSON keyframes with `at`, `speed` 0 to 16, and `easing`) for slow motion, fast
  forward, freezes, and speed ramps. Source time is the integral of the rate; a video's sound follows its
  picture. Audio is varispeed through an antialiased windowed-sinc reader. `check` reports `speed`,
  `mediaEnd` (videos), and `mediaSeconds` (audio clips). See `examples/speed.html`.
- `--quality draft|standard|high|max` (CLI), `"quality"` in `aether.json`, and `quality` in `aether_render`:
  H.264 quality presets with fixed quantizers 30, 26, 22, and 18 (CRF 28, 25, 21, and 18 with
  `--encoder ffmpeg`). `standard` is the default and byte-identical to earlier exports.
- `filter` color functions (`brightness`, `contrast`, `saturate`, `grayscale`, `sepia`, `invert`,
  `opacity`, `hue-rotate`) and `drop-shadow()`, after an optional `blur()`. On the root, color functions
  grade every finished frame.
- `mix-blend-mode` with the separable modes (`multiply`, `screen`, `overlay`, `darken`, `lighten`,
  `color-dodge`, `color-burn`, `hard-light`, `soft-light`, `difference`, `exclusion`) and `plus-lighter`.
- `data-motion-blur` on the root: deterministic shutter motion blur with `data-motion-blur-shutter`
  (degrees) and `data-motion-blur-samples` (automatic from the fastest motion when omitted).
- Animated filters: `blur`, `brightness`, `contrast`, `saturate`, `grayscale`, `sepia`, `hueRotate`, and
  `invert` in `data-hf-keyframes`, and `filter` in CSS `@keyframes`. An animated blur uses a continuous
  exact-variance kernel, so it eases without steps.
- `check` reports `effects` (motion blur settings, root grade, color filters, blend modes, and animated
  filters).
- `examples/effects.html` and `examples/filter-motion.html`.
- Composition variables: typed declarations in `data-composition-variables` (`string`, `number`, `color`,
  `boolean`, `enum`, `image`, `font`), bound with `var(--id)` in CSS, `data-var-text`, `data-var-src`, and
  `data-var-if`. Values come from `--variables` or `--variables-file` (`render`, `preview`, `check`,
  `bench`, `inspect`), `"variables"` in `aether.json`, or the `variables` argument of `aether_check`,
  `aether_write`, `aether_snapshot`, and `aether_render`. Unknown ids, wrong types, and out-of-range values
  are errors. `check` reports `variables` (values, defaults, uses, unused ids, removed elements). See
  `examples/variables.html`.

### Changed

- The encoder no longer pretends to use a `24..32` quantizer range: with rate control off OpenH264 always
  encoded at quantizer 26, which is now the explicit `standard` default, so default exports are unchanged.
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
