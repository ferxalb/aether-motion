# Aether — Agent Guide

## Language policy

All human-readable content in this repository must be in English, including documentation, comments, examples, CLI and MCP descriptions, error messages, and test messages. Write all new content in English and translate existing prose into English when editing it. Preserve code identifiers, protocol terms, and program behavior.

## Code organization and maintainability

- Keep the code modular. Group related modules into clearly named folders by domain or responsibility so files are easy to locate and recognize. Avoid generic catch-all folders and monolithic files that mix unrelated concerns.
- Split large files when they combine distinct responsibilities or become difficult to navigate. In particular, do not keep expanding `src/parse.rs` and `src/render.rs` as catch-all modules: extract cohesive areas into clearly named submodules and keep their entry points as small, easy-to-find facades.
- Give each module a focused responsibility and expose small, explicit interfaces so components can evolve independently. Keep boundaries clear between core logic, parsing, rendering, I/O, and external integrations.
- Start every source file with a concise English file-level comment describing its purpose, scope, and main responsibility. Use Rust module documentation (`//!`) for Rust files. Update this documentation when a file's responsibilities, interfaces, or place in the module structure changes; keep it useful without repeating implementation details.
- Keep related architecture and usage documentation in sync as the project grows. When a change adds or moves modules, update relevant file-level documentation and the repository's architecture or contributor guidance.
- Apply established design patterns when they fit the problem and the existing architecture. Prefer simple, reusable abstractions and low coupling; avoid patterns that add complexity without a concrete benefit.
- Treat runtime optimization separately from code organization: profile before making performance-focused changes, and do not claim a refactor is faster without measurements. Preserve deterministic output.
- Make changes with future scalability and maintainability in mind, keeping dependencies and responsibilities clear as the system grows.

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
- CSS animation is compiled to native motion tracks. Define `@keyframes name { from {...} 40% {...} to {...} }` using only `transform` (`translate`, `translateX`, `translateY`, uniform `scale`, `rotate`), `opacity`, and `animation-timing-function`. Apply it with `animation: name 1s ease-out 0.2s 1 normal forwards` or the `animation-*` longhands (`name`, `duration`, `delay`, `timing-function`, `iteration-count` as a positive integer or `infinite`, `direction`, `fill-mode`). One animation per element. The animation starts at the clip start plus `animation-delay`. CSS semantics apply: without `forwards`/`backwards` the element returns to its resting style outside the animation. Do not combine with `data-hf-keyframes` on the same element.
- Easing in CSS animations follows CSS: `linear`, `ease`, `ease-in`, `ease-out`, `ease-in-out` (cubic Béziers), `cubic-bezier(x1, y1, x2, y2)`, `steps(n, jump-start|jump-end)`, `step-start`, `step-end`. (`data-hf-keyframes` keeps its own quadratic easings.)
- `transform: translate(Xpx, Ypx) scale(s) rotate(a)` also works as a static style. Scale is uniform, grows around the element center, and scales descendants (positions, sizes, text, radius). `rotate(deg|rad|turn)` is clockwise around the center and can be animated like the rest. A rotated element is rendered on its own layer, so its descendants are clipped to its box plus its shadow reach. Skew, 3D, percent translations, and other transform origins are unsupported.
- Custom properties: declare `--name: value` on `:root` or any element (inherited) and use `var(--name, fallback)` in any supported value, including inside `@keyframes`.
- Colors: `#rgb`, `#rgba`, `#rrggbb`, `#rrggbbaa`, `rgb()`, `rgba()`, `hsl()`, `hsla()`, `transparent`, and a set of common names.
- Flex layout: `display: flex` with `flex-direction: row|column`, `gap` (px), `padding` (1 to 4 px values or `padding-top/right/bottom/left`), `justify-content: flex-start|center|flex-end|space-between|space-around|space-evenly`, and `align-items: flex-start|center|flex-end|stretch` (stretch is the default). Items use `width`/`height`, or their content size (text, nested flex containers); `flex-grow: n` and `flex: n` share the free space. All children occupy their slot for the whole composition, whatever their clip timing, so layout never jumps when a clip starts. Children with `position: absolute` stay out of the flow and use `left`/`top` as before. There is no wrap, shrink, margin, or `align-self`.
- `box-shadow: offset-x offset-y [blur [spread]] color`: a single soft outer shadow that follows `border-radius`, scale, and opacity. `inset` and multiple shadows are unsupported.
- `border: Npx solid color` (also `border-width`, `border-color`, `border-style: solid|none`): a uniform solid border that follows `border-radius`. Sizes are border-box, so the border sits inside `width`/`height` and insets content and flex children. The color defaults to white. Per-side, dashed, and dotted borders are unsupported.
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
