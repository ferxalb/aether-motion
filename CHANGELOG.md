# CHANGELOG

All notable changes to this project are documented here. The format follows
[Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses
[Semantic Versioning](https://semver.org/); while the version is `0.x`, the `data-hf-*` attributes and
CLI/MCP output may still change between minor releases.

## [Unreleased]

## [0.2.0] - 2026-11-21

### Added

- Native low-poly 3D (`data-hf-3d`, [docs/3d.md](docs/3d.md)): a sized `<div>` clip becomes a
  viewport whose padding box shows a scene graph of `<hf-node>` groups, `<hf-mesh>` geometry,
  `<hf-light>` lights, and one `<hf-camera>`, rasterized at output resolution and composited like
  any content (fades, filters, bloom, masks, motion blur, transitions, 2D cameras, and 2.5D planes
  apply). Coordinates follow CSS and 2.5D spaces (x right, y down, z toward the viewer; nodes
  compose as `translate3d rotateY rotateX rotate scale3d`).
  - Geometry: indexed triangle meshes (`shape="custom"` with `vertices` and `indices`) and
    procedural `plane`, `box`, `pyramid`, `cone`, `cylinder`, and icosphere `sphere` primitives.
  - Materials: flat Lambert (per-face, in linear light) and unlit, with a base `color` and
    `cull="back|none"`. Lights: `ambient` and `directional` (up to 8), keyframable `intensity`;
    a default rig when none is declared.
  - Camera: perspective (`perspective` focal length or `fov`) or orthographic, `zoom`, `yaw`,
    `pitch`, `roll` with the 2.5D camera's senses, `target` (a node or a point, with a level
    horizon), and `near`/`far`. Every node and the camera animate with `data-hf-keyframes`
    (`x`, `y`, `z`, `rotateX`, `rotateY`, `rotate`, `scale*`; camera `yaw`, `pitch`, `roll`,
    `zoom`), with the keyframe easings plus `smooth`.
  - Visibility: clip-space clipping against the near and far planes and a guard band, back-face
    culling, and a per-sample reversed depth buffer (ties keep document order).
  - CPU rasterizer (default): fixed-point edges with the top-left rule, an exact per-row span
    solver, `data-hf-3d-antialias` supersampling (1 to 4 per axis, default 2), and bounded 64 px
    tiles; deterministic across threads, segments, and CPUs.
  - wgpu backend included by default (`render --3d-backend gpu|auto` or
    `AETHER_3D_BACKEND`): the same shaded triangles, clip matrix, reversed depth, and integer
    resolve, within a documented tolerance of the CPU, with an automatic and sticky CPU fallback
    (stderr note, `check`'s `gpuFallback`).
  - `check` reports `scenes3d`; `AETHER_PROFILE=1` reports a `scene-3d` phase; root and
    per-element motion blur treat scene animation as movement.
  - `examples/lighthouse-isle-3d.html`, `tests/scene3d.rs`, and `tests/scene3d_gpu.rs`.
  [Benchmark details](BENCHMARK.md#020-native-low-poly-3d-2026-11-21)
- 2.5D spaces: `data-hf-space="perspective|orthographic"` (with `data-hf-perspective`,
  default 1000 px) turns a container into a 3D stage of flat planes. Planes take `data-hf-z`,
  `data-hf-rotate-x`, `data-hf-rotate-y`, and `data-hf-backface="hidden"`, keyframed as `z`,
  `rotateX`, and `rotateY` in `data-hf-keyframes`, and compose hierarchically with their layout,
  2D pose, and parent planes (CSS `translate3d rotateY rotateX rotate scale` order).
  - Camera: `data-hf-camera` (classic or rig keyframes, `smooth` included) or `data-hf-shots` on
    the space move a 3D camera: `x`/`y` (identical to a classic camera at depth 0), `zoom`, a `z`
    dolly, and `rotateX`/`rotateY`/`rotate` (tilt, pan, roll around the eye). Shots take the same
    values at their end and in `from`. `data-hf-camera-subject` follows a subject in 3D, dividing
    the anchor by its baked depth magnification (`camera::Gain`), and `data-hf-parallax` children
    pan by their factor.
  - Exact depth order: a BSP tree built in paint order (`space/bsp.rs`) draws planes at one depth
    like 2D siblings and splits intersecting planes where they cross; the pieces share a hard
    edge (each pixel belongs to one side), so no backdrop shows through the cut. Past 4096 pieces or
    256 levels it falls back to sorting whole planes by distance.
  - Drawing: whole planes facing an unturned camera are scaled, rolled 2D draws with every effect.
    Tilted planes, planes seen by a turned camera, and split pieces are warped from one layer per
    plane that covers its visible part (near and frame edges clipped in plane coordinates) and
    every flat descendant, with their shadows, glow, and motion. Minified areas are sampled
    trilinearly from on-demand mipmaps. Text on projected planes is reported to the layout probe
    through the projection.
  - Nesting and flattening: a nested `data-hf-space` is a plane of the enclosing space with its own
    camera, and an element inside a space that draws its subtree on a layer (`overflow: hidden`,
    filters, masks, blend modes, reveals, bloom, pixel effects, motion blur) flattens its planes
    into a nested space, as CSS does.
  - Motion blur: `data-hf-motion-blur` on a plane blurs its 3D motion; `layer_blur.rs` gained
    `blur_samples`, which redraws the element through a placement callback at every sample
    (visibility and back faces are decided per sample; a stationary blurred plane is split and
    ordered like any other).
  - Validation: a blend mode on a plane that always tilts or under a turning camera, and a camera
    subject inside a nested or flattening space, are rejected with errors naming the element.
  - Near clipping at a tenth of the perspective, scene instances, clip timing, fades, transitions,
    and root motion blur work as usual. A plane with 3D values needs a space ancestor, and 3D
    camera values need a space on the camera; other combinations are accepted.
  New modules: `src/space/` (transform, projection, and BSP math), `parse/space.rs`,
  `render/space.rs`, and `render/paint/project.rs`.
- `examples/depth-lantern-valley.html`: a six-second 1920×1080 perspective shot that trucks and
  dollies through a layered valley (one tilted lake that the ridges and reeds stand on, near-plane
  clipping, bloomed lanterns) and swings in a title card carrying a nested plane.
- `tests/space.rs`: projection against CSS perspective and the classic camera, orthographic
  foreshortening, nesting and depth order, split intersecting planes, uprights on a long floor,
  nested and flattened spaces, camera pan/tilt/roll, smooth and shot dollies, parallax, subject
  following in depth, per-plane motion blur, probed projected text, overflowing children and
  mipmapped minification, near clipping, back faces, timing, keyframed depth, scene instances,
  identical frames across render order, threads, and segments, and errors. Unit tests cover the
  view math (`space/math.rs`) and the BSP order (`space/bsp.rs`).
  [Benchmark details](BENCHMARK.md#25d-spaces-2026-11-20)
- Camera Rig v1: `data-hf-camera-subject` follows a descendant of a camera, held at
  `data-hf-camera-anchor`, with `-lag`, `-lookahead`, `-deadzone`, `-offset`, `-axis`, and
  `-easing` options. Tracking is baked once at load on a fixed 240 Hz grid and read back as a
  pure function of time, so frames match in any render order, thread count, and split. Subjects
  inside `data-hf-parallax` planes are framed with their plane's pan.
- The `smooth` camera easing: monotone cubic Hermite curves with automatic tangents and
  continuous velocity, no overshoot, and rest at the end keys. `data-hf-camera` keyframes also
  accept `anchorX`, `anchorY`, and `follow` (a weight that blends from keyframed focus to
  subject framing). On rig cameras, zoom interpolates in log space.
- `data-hf-shots`, a minimal shot composer: `static`, `follow`, `pan`, `dolly-in`,
  `dolly-out`, and `reveal` presets played back to back on one continuous rig, with `duration`,
  `zoom`, `to`, `anchor`, `follow`, `easing`, `blend`, and an opening pose (`from`). Invalid shots
  and rig options are rejected with specific errors and a fix hint.
- `check` and `aether_check` report rig cameras under `cameras`: the subject, its settings,
  and the resolved zoom, focus, anchor, and follow weight at every key.
- `examples/cinematic-camera-rig-6s.html`: a six-second 1920×1080 shot on The Morning Shift's
  street that tracks the walker, dollies in, and reveals the skyline. It reuses the short's
  artwork; the 15-second short is unchanged.
- `tests/camera_rig.rs`: rendered anchor framing through zoom and parallax, the demo's
  velocity continuity and end poses, identical frames across render order, thread count, and
  segmented export, unchanged classic cameras, and rig diagnostics.
- `tests/svg_zoom.rs`: zooming SVG frames match in any render order and from a cold renderer,
  frames at rest equal a camera that never zooms, a resampled frame stays within a measured bound
  of the exact raster without shifting, `contain` and `cover` stay exact while zooming, and a vector
  fit beyond the 64 MiB per-image limit renders its visible part.
- `svg::tests::a_region_matches_the_same_part_of_the_whole_raster` checks a region raster
  against the whole raster: within 1 everywhere, except under a path clipped at the region edge,
  which may differ by exactly one antialiasing step (16 of 255).
- Motion Scenes v1.2: `preview --watch` keeps compiled scenes between renders (`SceneCache`,
  `parse_file_cached`) and recompiles only scenes whose files changed, appeared, or disappeared,
  including every scene that imports a changed one. After a failed render it also watches the
  paths the attempt tried to read, so creating a missing scene or asset triggers a new render.
- `benchmarks/scene-reuse/`, the 144-instance scene reuse fixture, and `tests/scene_sharing.rs`.

### Changed

- Motion Scenes v1.2: instances of a reused scene share its compiled elements. `model::Element`
  keeps its id, tree links, and clip times and holds the rest in a shared copy-on-write
  `ElementBody` (fields read through `Deref`, so `element.style` works as before). The renderer
  shares text layouts and glyph rasters between instances, and skips everything drawn before a
  scene that covers the frame with an opaque, directly drawn background. On the 144-instance
  fixture, peak memory fell from 65 to 36 MB and overlapping frames render about 4x faster;
  output is byte-identical. A scene without `data-hf-split-wrap` text compiles once for both
  parses. [Benchmark details](BENCHMARK.md#motion-scenes-v12-shared-scenes-and-occlusion-2026-11-20)
- Cameras that use no rig feature keep their exact classic zoom and pan tracks. `Motion` gains an
  optional `rig`, and the parallax pan is read through `Motion::camera_pan`.
- Zoom-aware SVG reuse: while a camera or scale animation zooms `object-fit: fill` vector
  artwork, each frame is resampled (fixed-point Catmull-Rom, premultiplied) from a shared
  zoom-ladder raster at most one 2^(1/8) step above the drawn size, instead of being rasterized at
  every passing size. Levels live in the existing 64 MiB fitted-image cache and its LZ4 packing;
  frames at rest, `contain`/`cover` fits, and raster images are unchanged. On the Camera Rig demo
  (1080p, 144 frames) SVG rasterizations fell from 1,683 to 128 and the export from 184.5 s to
  57.0 s at one thread (58.9 s to 22.9 s at four); a 4K zoom window fell from 67.8 s to 23.6 s.
  Zooming frames differ from exact rasters by antialiasing only (PSNR 38–43 dB, edge energy within
  2%) and are temporally steadier: no shimmer or shift at level switches. SVG `<pattern>` fills
  (resvg rounds the pattern tile to whole pixels) re-phase at each level switch rather than on
  every zooming frame. `AETHER_PROFILE=1` reports `image-resample` time and `visible-region rasters`.
  [Benchmark details](BENCHMARK.md#zoom-aware-svg-reuse-2026-11-19)
- CLI redesign. `aether-motion --help` groups the commands (create and export, compose, coding
  agents, diagnose) and every command has its own help page with grouped options, defaults, and
  examples: `aether-motion help <command>` or `aether-motion <command> --help` (`-h`). `-V` and
  `version` print the version.
- One output style for every human-facing command: marked status lines (`✓`, `✗`, `!`, `+`, `~`,
  `=`), section headings, aligned facts, and paths without a leading `./`. Colors are used only on
  a terminal and follow `NO_COLOR` and `CLICOLOR_FORCE`. `doctor` reports by section with a fix
  under each problem and a final verdict; `init` separates project files from agent setup;
  `inspect` prints aligned tables; `bench` reports the frames it drew and the time per frame; the
  terminal render progress line shows a bar. Machine-readable output is unchanged: `check` JSON,
  `--progress json`, `block <name>` HTML, and `captions` to stdout.
- Errors print as `error: ...` on stderr. Command-line mistakes (unknown commands or options,
  missing or invalid values) exit with code 2, suggest the closest command or option, and point at
  the command's help; other failures exit with code 1.
- `render` has two explicit modes, chosen by whether a file is named, and `-o` and `--output` are
  one option in both. `render` without a file is project mode: the entry, fps, size, quality,
  output folder, and variable values come from `aether.json`. `render <file>` is direct mode:
  only the command line counts and `aether.json` is never read. Both modes take every option
  (`--frames`, `--encoder`, `--priority`, `--segments`, and `--parts` work in project mode too),
  with the same validation, messages, and exit codes; the flags of `render`, `inspect`, and
  `bench` are parsed once, in `src/cli/options.rs`.
- One collision rule for both modes: a PNG is overwritten; an existing MP4 or `.aseg` named with
  `-o`/`--output` needs `--force` (`-f`) and is refused with exit code 2 before the composition is
  parsed; the default output (`<out>/<name>.mp4`) is replaced on every render. A failing
  composition never removes the previous output. The output folder is created when missing.
- Migration: `render <file> -o <path>` (and `render <file>` with no output) used to apply
  `aether.json`; it now ignores it. Run `render` without a file to use the project, or pass
  `--size`, `--fps`, `--quality`, and `--variables-file` explicitly. `render <file> --output <png>`
  keeps overwriting stills, and `render <file> --output <mp4>` still refuses an existing file,
  now with exit code 2 and a `--force` hint (previously exit code 1 and no way to replace it).
  Project mode `-o <png>` no longer needs `--force`. `render <file>` without an output writes
  `out/<name>.mp4` in the current directory.
- `check` and `preview` follow the same rule: with a file they run standalone and never read
  `aether.json` (no project variables; `preview` at 30 fps), and without one they use the
  project's entry, variables, and fps. `preview` gains `--fps`. The `check` JSON report is
  unchanged. Both reject extra positional arguments, and `preview` reports invalid
  `--variables` as usage errors (exit code 2), as `check` and `render` do.
  Migration: `check <file>` and `preview <file>` inside a project no longer apply its
  `"variables"` or fps; drop the file or pass `--variables-file`/`--fps`.
- Replacing an output is now safe: `--force` and the automatic project export write the new file
  to a hidden sibling and rename it over the previous one only after the render and encode
  succeed, and PNGs (`render`, `--frames`, `preview`) are written to a temporary file first. A
  failed render, encode, or write leaves the previous output intact and no staging files behind.
  Previously `--force` and the automatic export removed the old file before rendering.
- `render` validates `--threads` (1 to 64) and `--audio-bitrate` (64 to 320) in both modes,
  rejects `--frame` for MP4 output, out-of-range frames, and `--encoder ffmpeg` for PNG, part,
  or assembled output, and checks its flags before parsing the composition. `init`, `render`,
  `inspect`, and `bench` reject extra positional arguments instead of ignoring them.
- `src/main.rs` only calls `cli::main`; `render` lives in `src/cli/render/` (`output.rs` holds the
  collision rule, `ffmpeg.rs` the optional FFmpeg encoder), and `inspect` and `bench` have their
  own files.
- Regression tests in `tests/cli.rs` run the binary for option equivalence, project isolation of
  `render`, `check`, and `preview`, broken configurations, the `check` JSON report, overwrite
  behavior, safe replacement on failure, invalid arguments, and exit codes.

### Fixed

- A vector fit larger than the 64 MiB per-image limit (deep camera zooms at 4K) is rasterized for
  its visible part instead of failing with `rasterized image exceeds 64 MiB`; a visible part above
  64 MiB still asks for `--scale`.

- The root shutter (`data-motion-blur`) now draws each sample at its own composition time, so
  animated clip paths and self-drawing strokes are averaged over the window instead of being
  drawn at the frame time in every sample.
- A per-element motion blur sample that fails to draw no longer leaves its pixels on the frame.
- GitHub Actions stages local npm tarballs using explicit file paths, so npm resolves the release
  archives from disk.

## [0.1.7] - 2026-11-19

### Added

- Native PNG and WebP cel playback with clip-local timing, hold, loop, pingpong, and bounded image caching.
- Camera-relative pan per depth plane and optional filmic root exposure, highlight roll-off, and shadow toe.
- The Morning Shift, an original layered industrial dawn example with illustrated assets, animated rain and fog, and a 12 FPS character sequence.
- The Morning Shift — Into the Shadows, a 15-second three-shot short with an original 12-cel
  walking cycle, world-space character lighting, foreground occlusion, and animated wet-pavement
  reflections. The example includes the new illustrations, cels, and their drawing source.
- `AETHER_PROFILE=1` now reports fitted-image hits, misses, evictions, packed entries, and
  raster and cache byte counts. It also times SVG parsing and rasterization, image fitting,
  packing, blitting (including alpha blending and unpacking), and frame finishing.
- `AETHER_IMAGE_PACK=off` selects the raw fitted-image cache for profiling and output comparisons.

### Changed

- Recut Into the Shadows with independently drawn side-elevation and lamp-yard plates,
  constant-scale tracking, heel and toe pivots, and synchronized sole shadows. Refined
  the street-space lighting masks, reflections, haze, and rain for the 1080p revision.
- Into the Shadows hides both shot changes: a motion-blurred foreground column carries a
  linear `wipe-left` at 5 s and a lamp-lit fog veil covers a 0.14 s crossfade at 10 s.
  A regression test renders both windows of the example and checks that the wipe edge
  stays under the column, that the crossfade happens under the fog, and that cold,
  forked, and reverse-order renders match byte for byte. The 4K master was validated
  frame by frame (see `BENCHMARK.md`).
- Extended the cinematic validation with a 15-frame 4K benchmark, an animated-zoom probe,
  native MP4 cadence checks, and byte-identical comparisons with the original continuity PNGs.
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
