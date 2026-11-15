# Aether

Aether is a local Rust tool for generating PNG frames and H.264 MP4 videos from timed HTML compositions. Its native backend supports the profile described below, renders without launching a browser or Node.js, and returns explicit errors for unsupported features. The native MP4 encoder includes OpenH264; FFmpeg is optional. Licensed under Apache 2.0.

## Getting started

Stable Rust is required to build the project. The native MP4 path includes OpenH264 and does not require FFmpeg. FFmpeg is available as an optional encoder with `--encoder ffmpeg`.

```bash
cargo build --release
cargo run --release -- inspect examples/title-card.html
cargo run --release -- render examples/title-card.html --output /tmp/title.png --frame 24 --fps 24 --scale 0.5
cargo run --release -- render examples/title-card.html --output /tmp/title.mp4 --fps 24 --scale 0.5 --threads 2
# Optional, if FFmpeg is already installed:
cargo run --release -- render examples/title-card.html --output /tmp/title-ffmpeg.mp4 --encoder ffmpeg
```

`--scale 0.5` halves the width and height, so each frame contains one quarter as many pixels. For native exports at 1080p or higher, `--threads 2` or more overlaps drawing and color conversion with encoding through a one-frame queue. Both encoders produce H.264/yuv420p; FFmpeg mode uses `veryfast` and CRF 25. The renderer keeps one RGBA frame, up to 64 MiB of scaled images, and up to 32 MiB of text masks. When the visible state matches the previous frame, it reuses the RGBA frame and the native encoder's YUV conversion.

## Supported input

| Input | Status |
| --- | --- |
| `data-composition-id`, `data-width`, `data-height`, `data-duration` | Yes; on the root element or in `<meta>` |
| Numeric `data-start` or a reference to another `id`, with an optional `+`/`-` offset | Yes |
| `data-track-index`, clips using `<div>`, text, and local PNG/JPEG/WebP images | Yes |
| Type, class, ID, and descendant CSS selectors; inline styles | Yes |
| `left`, `top`, `width`, `height` in px or %, hex/basic named colors, opacity, fonts, centered flex layout, `object-fit` | Yes |
| `border-radius` and `linear-gradient(...)` | Background radius in px and vertical or horizontal gradients with two opaque colors |
| `data-hf-fade-in`, `data-hf-fade-out` | Native extension; seconds, linear transition |
| `data-hf-keyframes` | Native extension; interpolated position and opacity with four easing curves |
| GSAP, `<script>`, `<video>`, `<audio>`, `<svg>`, canvas, complex effects, external compositions, general CSS | Not yet; explicit error |

Fonts are looked up on the system. Text wraps at spaces (`max-width`, `line-height`, `white-space: nowrap`) but does not implement all browser typography shaping: no kerning, hyphenation, or breaks inside words. `text-shadow` takes a single shadow. Only local images are accepted. For a real composition, run `inspect` first; if a feature is unsupported, the program reports the first incompatibility.

## Export at 1080p and 4K

```bash
aether-hyperframes render examples/motion.html --size 1080p --fps 60 --output /tmp/motion-1080p.mp4
aether-hyperframes render examples/motion.html --size 4k --fps 30 --output /tmp/motion-4k.mp4
```

The presets produce 1920×1080 and 3840×2160 from 16:9 compositions. Both support 30 and 60 fps. For other aspect ratios, use `--scale` or set the output dimensions in the HTML. Native MP4 supports up to 60 fps and even dimensions up to 3840×2160 or 2160×3840. The file uses a constant frame rate. Export time depends on the scene and the machine.

In MCP, the same presets are available through `size`. Use either `size` or `scale` in each call.

## Create and review animations

[The motion example](examples/motion.html) combines animated entrances and exits, layers, and relative timing. Keyframes use seconds relative to the clip. The attribute supports `x`, `y`, `opacity`, and `easing`. See the full contract in [AGENTS.md](AGENTS.md).

```bash
aether-hyperframes check examples/motion.html
aether-hyperframes render examples/motion.html --output /tmp/motion.png --fps 30 --frame 12
aether-hyperframes render examples/motion.html --output /tmp/motion.mp4 --fps 30
```

For agents, `aether-hyperframes mcp` exposes the contract through `aether_guide`. `aether_check` returns resolved times and local keyframe times in `animations`, along with an error and a suggested fix on failure. `aether_snapshot` returns a PNG showing the composition at the requested second.

## Benchmark on your machine

```bash
cargo run --release -- bench examples/title-card.html --fps 24 --scale 0.5
```

`bench` measures in-memory drawing, **without MP4 encoding**. The output shows elapsed time and the number of rasterized and reused frames. To measure the full pipeline, use `render` to produce an MP4. To compare engines, render the same supported composition with identical resolution, FPS, and encoding options on the same machine. Record total time, peak memory, and output size; do not extrapolate results from one machine to another.

[Initial measurements](BENCHMARK.md).

The code uses portable Rust APIs, and the repository includes a CI matrix for Linux, macOS, and Windows.

## Next steps

1. Add more CSS properties and layout cases from real compositions, with visual fixtures.
2. Measure the full MP4 pipeline on Windows and Linux as well as macOS, and profile CPU and memory by stage.
3. Evaluate OpenH264 quality and output size against FFmpeg on real scenes.

## Source layout

- `src/model.rs`: parsed composition, styles, and time-sampled motion tracks.
- `src/parse/`: HTML/CSS parser; unsupported features fail with explicit errors. `mod.rs` is the `parse_file` facade; `css/` (stylesheet, selectors, cascade, variables), `values/` (lengths, colors, backgrounds, transforms, shadows), `style/` (property groups: layout, text, visual), `animation/` (CSS animation and `data-hf-keyframes` into motion tracks), `tree.rs` (DOM walk), `timeline.rs` (`data-start`), `html.rs` (node helpers).
- `src/render/`: software renderer. `mod.rs` holds the public `Renderer` and frame loop; `draw.rs` walks the scene; `layout/` sizes and places flex items; `paint/` holds pixel primitives (fills, borders, shadows, images, rotation); `text/` handles fonts, measurement, and glyph painting; `geometry.rs` and `frame_state.rs` hold small shared types.
- `src/encode.rs`: native H.264/MP4 encoding. `src/agent.rs`: `check` and the MCP server.
