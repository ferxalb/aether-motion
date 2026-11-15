# Aether — Agent Guide

Create, validate, inspect, and render HTML compositions. Output must be deterministic: what appears in a frame is a function of time.

## Work loop

1. Run `aether-hyperframes check <file.html>`. It returns JSON with `ok`, structure, and, on failure, `error` and `fix`. Continue until `ok: true`.
2. Inspect the result with `aether-hyperframes render <file.html> --output frame.png --frame N` or the `aether_snapshot` MCP tool.
3. For 16:9 compositions, choose `--size 1080p` or `--size 4k` and `--fps 30` or `--fps 60`. The native encoder writes one MP4 sample per frame.
4. Render the video with `aether-hyperframes render <file.html> --output video.mp4`.

## Supported contract

- The root has `data-composition-id`, `data-width`, `data-height`, and `data-duration`, either on the root element or on `<meta>`.
- Each clip uses `class="clip"`, `data-start` (seconds or another clip's `id`, optionally with a `+` or `-` offset), `data-duration`, and `data-track-index`.
- Supported elements include `div`, text, and local `img` files (PNG, JPEG, or WebP). Styles may be inline or use type, class, and ID selectors.
- Supported properties include `left`, `top`, `width`, and `height` in px or %, colors, `opacity`, `font-size` in px, `font-family`, `text-align`, centered flex layout, `object-fit`, and `z-index`.
- Draw rounded backgrounds with a single nonnegative `border-radius` in px. The radius affects the background of the element. Supported gradients use `background: linear-gradient(to right, #123, #456)` with exactly two opaque colors. Supported directions are `to right`, `to left`, `to top`, and `to bottom`. The default direction is down.
- Fades use `data-hf-fade-in` and `data-hf-fade-out`, in seconds.
- Use `data-hf-keyframes` for native motion. The single-quoted HTML attribute contains a JSON array, for example `data-hf-keyframes='[{"at":0,"x":-80,"opacity":0},{"at":0.6,"x":0,"opacity":1,"easing":"ease-out"}]'`.
- Each keyframe requires `at` in seconds from the element's resolved start. Times must be strictly increasing within its duration. `x` and `y` are offsets in composition pixels that move the element and its children. `opacity` (0–1) multiplies existing opacity and fades. Omitted values inherit the previous keyframe's values and default to x=0, y=0, opacity=1.
- The destination keyframe sets the segment's easing. Available values are `linear` (default), `ease-in` and `ease-out` (quadratic), and `ease-in-out` (smoothstep). The first and last values hold outside the keyed interval while the clip is active.
- Network access, `Date.now()`, and randomness are unsupported. Use relative paths.
- Unsupported features return an explicit error; never silently produce a wrong video. This includes `<script>`, GSAP, `<video>`, `<audio>`, `<svg>`, canvas, and general CSS.

## Benchmark privacy

- Preserve measured benchmark and test results, including timings, memory use, output sizes, and comparisons. Do not delete or alter them merely to sanitize a document.
- Remove only private machine-identifying metadata: exact hardware models, local operating-system versions or builds, host architecture when it identifies the user's device, hostnames, usernames, serial numbers, and user-specific local paths. Describe the machine generically or omit that metadata.
- Keep the benchmark settings needed to interpret results. Never add identifying device details to tracked documentation.

## MCP

`aether-hyperframes mcp` speaks MCP over stdio. Tools: `aether_guide`, `aether_write`, `aether_check`, `aether_snapshot` (returns an image), and `aether_render`.

For motion, read the `animations` entries from `aether_check` and inspect the start, midpoint, and end of a transition with `aether_snapshot`. Its `t` argument is an absolute composition time in the range `0 <= t < duration`. See `examples/motion.html`.

Pass `size: "1080p"` or `size: "4k"` to `aether_snapshot` or `aether_render`. Specify either `size` or `scale`. For native MP4, use `fps` from 1 to 60 and `threads` from 1 to 64.
