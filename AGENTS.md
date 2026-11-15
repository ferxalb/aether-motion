# Aether — Agent Guide

Create, validate, inspect, and render HTML compositions. Output must be deterministic: what appears in a frame is a function of time.

## Work loop

1. Run `aether-hyperframes check <file.html>`. It returns JSON with `ok`, structure, and, on failure, `error` and `fix`. Continue until `ok: true`.
2. Inspect the result with `aether-hyperframes render <file.html> --output frame.png --frame N` or the `aether_snapshot` MCP tool.
3. Render the video with `aether-hyperframes render <file.html> --output video.mp4`.

## Supported contract

- The root has `data-composition-id`, `data-width`, `data-height`, and `data-duration`, either on the root element or on `<meta>`.
- Each clip uses `class="clip"`, `data-start` (seconds or another clip's `id`, optionally with a `+` or `-` offset), `data-duration`, and `data-track-index`.
- Supported elements include `div`, text, and local `img` files (PNG, JPEG, or WebP). Styles may be inline or use type, class, and ID selectors.
- Supported properties include `left`, `top`, `width`, and `height` in px or %, colors, `opacity`, `font-size` in px, `font-family`, `text-align`, centered flex layout, `object-fit`, and `z-index`.
- Fades use `data-hf-fade-in` and `data-hf-fade-out`, in seconds.
- Network access, `Date.now()`, and randomness are unsupported. Use relative paths.
- Unsupported features return an explicit error; never silently produce a wrong video. This includes `<script>`, GSAP, `<video>`, `<audio>`, `<svg>`, canvas, and general CSS.

## Benchmark privacy

- Preserve measured benchmark and test results, including timings, memory use, output sizes, and comparisons. Do not delete or alter them merely to sanitize a document.
- Remove only private machine-identifying metadata: exact hardware models, local operating-system versions or builds, host architecture when it identifies the user's device, hostnames, usernames, serial numbers, and user-specific local paths. Describe the machine generically or omit that metadata.
- Keep the benchmark settings needed to interpret results. Never add identifying device details to tracked documentation.

## MCP

`aether-hyperframes mcp` speaks MCP over stdio. Tools: `aether_guide`, `aether_write`, `aether_check`, `aether_snapshot` (returns an image), and `aether_render`.
