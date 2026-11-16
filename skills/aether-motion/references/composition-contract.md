# AETHER Motion composition contract

This is a portable summary for authoring compositions. In the repository, `AGENTS.md` is authoritative and contains the full supported property and attribute list.

## Root and clips

The composition root declares `data-composition-id`, `data-width`, `data-height`, and `data-duration` on the root element or through supported `<meta>` declarations. A timed visual uses `class="clip"`, `data-start`, `data-duration`, and `data-track-index`. Starts may be seconds or another clip's ID with an optional `+` or `-` offset. Use IDs to make dependencies readable.

```html
<div id="stage" data-composition-id="launch" data-width="1920" data-height="1080"
     data-duration="4" style="position: relative; background: #101522">
  <div id="title" class="clip" data-start="0" data-duration="4" data-track-index="0"
       style="position: absolute; left: 160px; top: 360px; width: 1600px; height: 200px;
              display: flex; align-items: center; justify-content: center;
              color: white; font-size: 96px; font-weight: bold">
    Product launch
  </div>
</div>
```

Use supported elements (`div`, text, local PNG, JPEG, WebP, or SVG images, and inline static `<svg>`), inline styles or type/class/ID selectors, and documented CSS properties only. Layout dimensions accept pixels or percentages. Flex layout is available for centered and grouped content; do not rely on full browser layout behavior.

## Deterministic motion

Use the native motion attributes for keyframes, camera moves, split text, reveals, counters, and scene transitions. Keyframe times are seconds relative to the element's resolved start and must increase strictly within the clip duration. Keep all animation seekable from time alone: no `<script>`, timers, network, `Date.now()`, or unseeded randomness.

CSS animation is limited to the documented transform, opacity, and timing-function subset. Static transforms support translation, uniform scale, and rotation. Unsupported CSS and HTML return explicit errors; do not replace unsupported behavior with a browser-only implementation and claim the native renderer supports it.

## Assets and audio

Use relative paths to local PNG, JPEG, WebP, or SVG images. SVG (file or inline `<svg>`) needs an explicit width and height, is rasterized at the target size, and must be static: no `<text>`, SMIL, scripts, `<foreignObject>`, or external references. `<audio>` supports local WAV, FLAC, Ogg Vorbis, and MP3 sources, with documented timing, trim, loop, volume, and fades. Audio mixing uses the native encoder; `--encoder ffmpeg` rejects compositions containing `<audio>`. Keep the checked audio peak at or below 1.0 to avoid clamping.

## Source of truth

For the exact supported gradients, shadows, text layout, clipping, CSS properties, motion options, and errors, consult the repository's `AGENTS.md` and `limits.md`. The format originates from [HyperFrames](https://github.com/heygen-com/hyperframes), but AETHER Motion supports only its own documented subset.
