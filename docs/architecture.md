# AETHER Motion architecture

Source layout and resource limits of the native renderer. Contributor rules (CPU budget, benchmarking, tests) live in [AGENTS.md](../AGENTS.md); measurements live in [BENCHMARK.md](../BENCHMARK.md).

## Source layout

- `src/model.rs`: parsed composition, styles, and time-sampled motion tracks.
- `src/parse/`: HTML/CSS parser; unsupported features fail with explicit errors. `mod.rs` is the `parse_file` facade; `css/` (stylesheet, selectors, cascade, variables), `values/` (lengths, colors, backgrounds, transforms, shadows), `style/` (layout, text, and visual property groups), `animation/` (CSS animation, `data-hf-keyframes`, and `data-hf-camera` into motion tracks), `split.rs` (`data-hf-split` kinetic text), `transition.rs` (`data-hf-transition` scene transitions), `tree.rs` (DOM walk), `timeline.rs` (`data-start`), `html.rs` (node helpers).
- `src/render/`: software renderer. `mod.rs` holds the public `Renderer` and frame loop; `draw.rs` walks the scene; `layout/` sizes and places flex items; `paint/` holds the `Surface` pixel buffer and its primitives (fills, gradients and soft shadows backed by a memory-bounded tile cache in `tile_cache.rs`, borders, outer and inset box shadows, backdrop and element blur, images with cached row spans, rotation, and the vignette/grain `finish` pass); `text/` handles fonts, inherited properties, line wrapping (`wrap.rs`), block layout (`block.rs`), glyph painting with fixed-size coverage masks and subpixel resampling (`mask.rs`) for continuous translation/zoom, and `text-shadow` (`shadow.rs`); `allocation.rs` checks frame/layer byte sizes and coordinate conversions; `geometry.rs` and `frame_state.rs` hold small shared types.
- `src/blocks/` and `blocks/`: ready-made blocks and themes. `blocks/*.html` are `{{name|default}}` templates embedded by `catalog.rs`; `template.rs` fills them; `mod.rs` serves the `block` CLI command and the `aether_block` MCP tool.
- `src/encode.rs`: native H.264/MP4 encoding. `src/agent.rs`: `check` and the MCP server.

## Memory and resource limits

The renderer keeps one RGBA frame, up to 64 MiB of scaled images, and up to 32 MiB of text masks. When the visible state matches the previous frame, it reuses the RGBA frame and the native encoder's YUV conversion.

Each gradient or outer-shadow tile cache retains at most 4,096 keys, including first sightings and rejected tiles. Exceeding that metadata limit returns an explicit render error; existing slots are not evicted or changed, preserving the cached/direct drawing decisions. Reduce animated size variation or split the composition when this limit is reached. Fitted-image and text-run caches also retain at most 4,096 entries; image eviction and uncached text preparation retain the same pixel calculations. Text-layout blocks already have a 4,096-key cap.

Frames and offscreen RGBA layers keep their 128 MiB per-buffer limit, with checked multiplication and widened coordinate subtraction before allocation. Rotation and element blur reject unrepresentable pixel coordinates. Shadow-edge profiles are limited to 16 MiB before allocation. These are per-resource bounds, not a guarantee of total process memory or successful allocation under memory pressure.

## Encoding

Both encoders produce H.264/yuv420p at a constant frame rate. The native encoder (OpenH264) is built in; `--encoder ffmpeg` uses FFmpeg from `PATH` with `veryfast` and CRF 25. For native exports at 1080p or higher, `--threads 2` or more overlaps drawing and color conversion with encoding through a one-frame queue.
