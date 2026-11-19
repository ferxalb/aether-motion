# CHANGELOG

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/); while the version is `0.x`, the `data-hf-*` attributes and
CLI/MCP output may still change between minor releases.

## [Unreleased]

### Added

- Native PNG and WebP cel playback with clip-local timing, hold, loop, pingpong, and bounded image caching.
- Camera-relative pan per depth plane and optional filmic root exposure, highlight roll-off, and shadow toe.
- The Morning Shift, an original layered industrial dawn example with illustrated assets, animated rain and fog, and a 12 FPS character sequence.
- `AETHER_PROFILE=1` now reports fitted-image hits, misses, evictions, packed entries, and
  raster and cache byte counts. It also times SVG parsing and rasterization, image fitting,
  packing, blitting (including alpha blending and unpacking), and frame finishing.
- `AETHER_IMAGE_PACK=off` selects the raw fitted-image cache for profiling and output comparisons.

### Changed

- Under byte pressure, the fitted-image cache stores compressible SVG rasters in lossless LZ4
  row bands within the existing 64 MiB budget. Painting unpacks only the bands it needs into a
  reusable 256 KiB buffer. Cache keys, bitmap and sequence storage, and rendered pixels stay the same.
- On the native macOS cinematic benchmark, rendering three 4K PNGs took 6.21 s, down from
  15.30 s. CPU time fell 59.7% and peak RSS fell 31.1%. The isolated 4K still used 33.7% less
  peak RSS and took 0.12 s longer. The 1080p run was effectively unchanged.
  [Benchmark details](BENCHMARK.md#fitted-svg-cache-pressure-at-4k)
- Added regression coverage for packed row bands, clipped alpha blits, and partial bands.
  The three 4K continuity PNGs matched byte-for-byte, and a cache-pressure fixture produced
  identical segmented exports at 1, 2, 4, and 64 configured threads.

## [0.1.6] - 2026-11-18

### Added

- Kinetic text: `data-hf-split` takes start values that combine channel by channel
  (`data-hf-split-x`, `-y`, `-scale`, `-rotate`, `-opacity`) over the `data-hf-split-from` preset,
  stagger orders (`data-hf-split-order="forward|reverse|center|edges"`), and masked reveals per word
  or per line (`data-hf-split-mask="words|lines"`). `data-hf-split-wrap` wraps lines automatically at
  the element's width with the renderer's own fonts and layout (the composition is parsed twice), and
  wrapping never changes the pieces or their timing. Lines animate with `data-hf-split-line-x`, `-y`,
  `-scale`, `-opacity`, `-stagger`, `-delay`, `-duration`, and `-easing`, with or without wrapping.
  Existing split compositions build the same elements as before. See `examples/kinetic-wrap.html`.
- Spring easing: `spring`, `spring(overshoot)`, and `spring(overshoot, oscillations)` wherever
  `data-hf-keyframes` easings are accepted (keyframes, camera, splits, reveals, draws, motion paths,
  counters, speed and volume ramps). The highest value is exactly `1 + overshoot` (solved once at
  parse time), and the curve comes to rest with zero velocity at its keyframe.
- Localized bloom: `data-hf-bloom="<intensity>"` with `data-hf-bloom-threshold`, `-radius`, and `-tint`
  adds a halo of the element's bright pixels over the frame, keeping the element itself exactly as
  sharp. The halo is bounded by the element's effect layer, computed on a 2x or 4x reduced grid for
  wide radii, dithered, and its planes are reused across frames. `bloom` in `data-hf-keyframes`
  animates the intensity. See `examples/bloom.html`.
- Per-element motion blur: `data-hf-motion-blur="on"` (with `-shutter` and `-samples`) averages one
  subtree over a shutter window at the rendered frame rate while the rest of the frame is drawn once.
  Samples are drawn over the real background, so `backdrop-filter` and blend modes inside the subtree
  stay correct, and stills rendered with `--frame N --fps F` match the export at `F`. Combining it with
  the root's `data-motion-blur`, nesting it, or putting it on the root are errors with a fix. With only
  that element moving, it renders the same pixels as the root shutter with the same settings. See
  `examples/layer-motion-blur.html`.
- `aether_snapshot` takes an optional `fps` (1 to 240, validated like `--fps`; default 30, as before)
  for motion-blur shutter windows. A snapshot at `t = N / fps` goes through the same still path as
  `render --frame N --fps F` and returns the same pixels (`tests/mcp_snapshot.rs` decodes both PNGs).
  The project-style `render` command now checks `--fps` with the same range.
- `AETHER_PROFILE=1` prints the thread time of drawing, YUV conversion, encoding, effect layers,
  bloom, and per-element motion blur on stderr after a render (see `src/profile.rs`).
- `examples/aether-motion-016-demo.html`: a 10-second monochrome demo of the new kinetic text, bloom,
  and motion blur, based on the monochrome promo.

### Changed

- The vignette and grain pass processes rows in blocks of eight pixels with plain loops that inline
  into its AVX2 copy (the per-row closures had been compiled without AVX2), hashes the block side by
  side, and divides the noise by 65535 with an exact 32-bit form. Output is byte-identical (also with
  `AETHER_SIMD=off`); the pass executes 7.4 times fewer instructions, and drawing
  `aether-monochrome-promo.html` at 4K60 is about 10% faster (see `BENCHMARK.md`).
- Glyph and text-shadow masks compute each destination column's source position once per paint
  instead of once per pixel, with the same arithmetic (byte-identical).

## [0.1.5] - 2026-11-17

### Added

- Typography: `font-kerning: normal` applies the font's pair kerning (GPOS `kern` pair adjustments or
  the legacy `kern` table; on the embedded Roboto it matches Chromium's shaping on every tested pair),
  including across the letters of `data-hf-split="chars"`. `font-style: italic|oblique` uses an
  installed italic face or slants the upright one by 14 degrees around the baseline, as browsers do
  (generic families always slant the embedded Roboto, so they match on every machine);
  `<em>` and `<i>` are italic, as elements and as inline runs. The default `font-kerning: auto` does
  not kern, so existing layouts are unchanged. See `examples/typography.html`.
- Multi-line kinetic text: `data-hf-split` accepts `<br>`; each line becomes a row that follows the
  element's `text-align`, and the stagger continues across lines.
- Render progress: `render` redraws a progress line with throughput and ETA on an interactive stderr;
  `--progress json` writes JSON lines (`done`, `total`, `fraction`, `fps`, `elapsedMs`, `etaMs`) and
  `--progress off` disables it. `aether_render` sends MCP `notifications/progress` when the request
  carries `_meta.progressToken`. The library exposes `ProgressObserver`, `ProgressUpdate`,
  `encode_native_with_progress`, and `render_part_with_progress`. Progress never changes the output.
- Distributed rendering: `render --output part.aseg --segments A..B` encodes a range of one-second
  segments into a part file, and `render --output video.mp4 --parts a.aseg,...` assembles parts (any
  order, checked for full coverage and matching settings) with the composition's audio, producing the MP4
  a single render writes. `inspect --fps N` prints the segment count. The library exposes `render_part`,
  `assemble_parts`, and `segment_count`. `render --frames 0,45,90` with `{frame}` in the PNG name writes
  several stills from one process. `encode.rs` is now `encode/` (`segments.rs`, `parts.rs`).

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

- Determinism: MP4 output is now byte-identical for every `--threads` value and every distributed
  split, including compositions with cached gradients and soft shadows (`premium-promo.html` used to
  differ between `--threads 1` and `4`). An uncached draw of a whole-pixel gradient or soft shadow now
  computes the same fixed-point texels a cached tile holds, so first draws and snapshots of those
  layers can differ from 0.1.0 by one level per channel (two in a few pixels inside filtered layers).
- Faster drawing with identical pixels: branch-free per-row kernels (`render/paint/kernels.rs`),
  compiled for both the baseline target and AVX2 and selected at run time, for two-stop gradients,
  radial gradient parameters, cached gradient tiles, shadow masks, and the vignette and grain pass;
  directly drawn radial gradients skip the transparent area outside their last stop. At 1080p on one
  thread: `premium-promo.html` −13%, `glass.html` −15%, drifting glows −9% to −20%, grain −54%,
  vignette −19% (median of three runs; see `BENCHMARK.md`). `AETHER_SIMD=off` forces the baseline
  kernels, for comparison.
- `<video>` sources with B-frames are rejected when the composition is parsed, with a re-encode
  command, instead of failing during rendering: the bundled OpenH264 decoder cannot decode them (the
  contract used to say B-frames were supported).
- The encoder no longer pretends to use a `24..32` quantizer range: with rate control off OpenH264 always
  encoded at quantizer 26, which is now the explicit `standard` default, so default exports are unchanged.
- AAC encoding is about 12 times faster (60 s of stereo audio: 16.7 s down to 1.4 s on one core) with
  byte-identical output: the vendored `oxideav-aac` computes its forward MDCT with an FFT.
- Gradient, shadow-mask, glyph, and small translucent fills avoid soft-float `round` and libm `hypot` calls in their
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
