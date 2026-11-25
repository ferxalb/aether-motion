# Procedural rendering roadmap

This document reviews the core renderer from the point of view of creative range, and plans the
procedural features that extend it. Phase 1 (procedural shaders) is implemented. See
[Shaders](shaders.md). Later phases are proposals, and each one is a self-contained delivery.

## Review of the core

The pipeline (`parse` → `render` → `encode`) is sound for a deterministic renderer.

- **Motion is compiled, frames are sampled.** CSS animation, keyframes, cameras, paths, and split
  text become tracks at parse time, so a frame is a pure function of time. New content types slot
  in as a `model::Content` variant that is parsed once and drawn by sampling at `t`.
- **Determinism is designed in, not tested in.** Fixed-point texels in the tile caches, branch-free
  kernels compiled for the baseline and AVX2 with bit-identical results, and one-second encode
  segments make output independent of threads, splits, and CPUs.
- **The frame loop already avoids work.** It skips identical frames, occlusion-culls siblings under
  opaque full-frame scenes, caches tiles on second sighting, and reuses fitted images. A new
  content type only has to report what changes its pixels (`FrameState`) to benefit.

Gaps that limited creative range before this work:

1. **No generative imagery in 2D.** Every pixel came from authored primitives: boxes, gradients,
   images, SVG, text, and video. Organic motion (noise fields, flowing color, grain with structure,
   dissolves with irregular edges) needed pre-rendered video or image sequences, which are large
   and cannot follow a theme or a variable.
2. **Shaders existed only in 3D.** `scene3d/look/shader.rs` has nine named surface shaders, but
   they are fixed functions on meshes and there was no way to write one.
3. **Blocks were fragments, not scenes.** The catalog had titles, cards, and ambient glows. It had
   no complete backgrounds or scenes and nothing that animated a surface.
4. **There were no data visualizations.** Charts had to be built from hand-placed boxes.

## Phase 1: procedural shaders (implemented)

`data-hf-shader` is a new `Content::Shader`: an expression program evaluated over the padding box.

- **Domain module `src/shader/`** (independent of HTML and pixels): `syntax.rs` (lexer and
  parser), `compile.rs` (type checking, lowering of vector operations to scalar SSA, constant
  folding with the runtime's own functions, common-subexpression sharing, dead-code removal,
  post-order scheduling, and register reuse), `vm.rs` (an interpreter whose registers hold 64
  points, so each instruction is one fixed-length loop that vectorizes; AVX2 build chosen at run
  time), and `math.rs` and `noise.rs` (`sin`, `exp2`, `log2`, `atan2`, gradient noise, fbm,
  voronoi, and hash built from IEEE operations and integer hashing only, so no system math library
  leaks platform differences).
- **Parsing** (`parse/shader.rs`): the program, uniforms (numbers, vectors, CSS colors, and
  `var()` for themes), keyframed uniforms with any keyframe easing, resolution, and dither.
- **Painting** (`render/paint/shader.rs`): the grid is placed over the visible part of the box,
  at full, half, or quarter resolution, and anchored to the box. Reduced grids are bilinearly
  interpolated in premultiplied space. Edges get fractional coverage, then opacity and an ordered
  dither are applied. The last grid of each element is kept with the values and placement that made it, under a
  64 MiB budget, so a still shader costs only its blend.
- **Frame identity**: `FrameState::shader` carries `Shader::key` (a hash of the frame values that
  vary), so the identical-frame shortcut and the shutters see shader animation.
- **Blocks**: `aurora`, `liquid-gradient`, `tech-grid` (backgrounds), `lower-third`, and
  `intro-scene` (complete scenes), with theme colors flowing into uniforms through `var()`.

## Phase 2: particles

A `data-hf-particles` content type for sparks, dust, snow, bokeh, confetti, and embers.

- **Model.** An emitter (point, line, box, or an element's outline), rate or burst, lifetime, and
  per-particle start ranges (velocity, size, color, spin) drawn from a counter-based hash of
  `(emitter seed, particle index)`. Forces are gravity, drag, and a curl-noise field (reusing
  `shader::noise`).
- **Determinism without simulation state.** Particle `i` is born at `i / rate`. Its state at `t` is
  either closed form (ballistic plus drag) or integrated on a fixed 120 Hz grid from its birth.
  Each frame computes only the particles alive at `t`, from scratch. Like `scene3d/physics.rs`,
  results never depend on which frames were rendered before.
- **Drawing.** Sprites are soft discs, squares, streaks along velocity, or a small image. Alpha
  splatting goes into a row-bucketed accumulator, reusing the `layer_blur.rs` integer-accumulator
  approach for exact order-independent sums. An optional shader program shades each particle
  (`life`, `speed`, and `id` as inputs).
- **Budget.** At most 100,000 live particles. Bucketing keeps the cost proportional to covered pixels.

## Phase 3: data and charts

Generated blocks (like the `rig/` characters, not templates) that take data as JSON or
composition variables.

- `bar-chart`, `line-chart` (a drawn-on path through `data-hf-draw` with an animated head),
  `donut` (a shader ring with `progress`), `kpi-grid` (counters), and `progress-steps`.
- Data comes from `--set data='[...]'` or a variables file, so one template renders a weekly
  report video. Axes, labels, and gridlines use the theme, and number formatting reuses
  `data-hf-counter`.
- `check` reports overflowed labels through the existing safe-zone probe.

## Phase 4: scene library

Complete, themed, timed scenes that follow `skills/aether-motion-cinematic`: logo reveal, product
spotlight, testimonial, before/after split, social vertical variants (9:16), and outro with
credits. Each scene is a standalone native HTML file usable through `data-composition-src`, with
`data-composition-variables` for its text and colors, so it composes with the existing scene
cache.

## Phase 5: performance headroom

Profile-driven work, measured with `bench` and recorded in `BENCHMARK.md`.

- **Shader tiling across threads.** A single frame's grid splits into independent row bands. Today
  parallelism comes from encode segments. A band split would speed up snapshots and previews
  without changing output.
- **Static subexpression hoisting.** Hoist instructions that depend only on `uv.y`, uniforms, or
  constants out of the per-chunk loop. It needs the same bit-identical guarantee, which holds
  because the operations are unchanged, only evaluated less often.
- **GPU evaluation** of shaders through the existing wgpu workspace as an opt-in backend, with
  the CPU VM as the reference. It needs exact IEEE parity for the subset used, or a documented
  tolerance like the reduced-resolution backdrop blur.
