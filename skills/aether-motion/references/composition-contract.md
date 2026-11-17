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

## Templates

To render one composition with different text, colors, sizes, or assets, declare typed variables on `<html>` or the root: `data-composition-variables='[{"id":"title","type":"string","label":"Title","default":"Launch day"},{"id":"accent","type":"color","label":"Accent","default":"#4be0b8"}]'` (types `string`, `number`, `color`, `boolean`, `enum`, `image`, `font`; `id`, `type`, `label`, and a typed `default` are required). Use them with `var(--accent)` in CSS (set on the root, over `:root` declarations; a number with `unit: "px"` arrives as `48px`), `data-var-text="title"` (the element's own text; child elements stay), `data-var-src="logo"` (on `img`, `video`, `audio`, `source`), and `data-var-if="showBadge"`, `"!showBadge"`, or `"layout=left"` (removes the element, or a `<style>`, when false). Values are passed per render as a JSON object (`--variables`, `--variables-file`, `aether.json` `"variables"`, or the MCP `variables` argument). Every mistake is an explicit error. Check and snapshot each value set. See `examples/variables.html`.

## Deterministic motion

Use the native motion attributes for keyframes, camera moves, split text, reveals, counters, captions (`data-hf-captions` with a local `.json`, `.srt`, or `.vtt` transcript; times are seconds from the clip start; see AGENTS.md for options), and scene transitions. Keyframe times are seconds relative to the element's resolved start and must increase strictly within the clip duration. Keep all animation seekable from time alone: no `<script>`, timers, network, `Date.now()`, or unseeded randomness.

For finish, use `filter` on an element in the fixed order `[blur()] [color functions] [drop-shadow()]` (for example `filter: saturate(1.2) drop-shadow(0 12px 24px rgba(0,0,0,.5))`), color functions alone on the root to grade every frame, `mix-blend-mode` (separable modes and `plus-lighter`, not inside `overflow: hidden`, rotated, revealed, filtered, or blended ancestors), and `data-motion-blur="on"` on the root for shutter motion blur (`data-motion-blur-shutter` in degrees, default 180; `data-motion-blur-samples="8"` for fast drafts). Filter amounts animate in `data-hf-keyframes` (`blur`, `brightness`, `contrast`, `saturate`, `grayscale`, `sepia`, `hueRotate`, `invert`, absolute values) and in CSS `@keyframes` `filter`, for focus-ins, color reveals, and flashes; `glitch` (0 to 1) and `rgbSplit` (px) in `data-hf-keyframes` add digital glitch accents. Scene transitions include `crossfade`, `push-*`, `cover-*`, `wipe-*`, `whip-*`, `zoom-in`, `zoom-out`, `dip-to-black`, `dip-to-white`, `flash`, `blur`, `glitch`, and `rgb-split`; see `examples/light-transitions.html`. Cut elements to shapes with `clip-path` (`inset()`, `circle()`, `ellipse()`, `polygon()` in px or %, animated through `clipPath` in `data-hf-keyframes` for iris, diamond, and unfold reveals) and fade them with a gradient `mask-image`; see `examples/clip-mask.html`. See `examples/effects.html` and `examples/filter-motion.html`.

CSS animation is limited to the documented transform, opacity, and timing-function subset. Static transforms support translation, uniform scale, and rotation. Unsupported CSS and HTML return explicit errors; do not replace unsupported behavior with a browser-only implementation and claim the native renderer supports it.

## Assets and audio

Use relative paths to local PNG, JPEG, WebP, or SVG images. SVG (file or inline `<svg>`) needs an explicit width and height, is rasterized at the target size, and must be static: no `<text>`, SMIL, scripts, `<foreignObject>`, or external references. `<audio>` supports local WAV, FLAC, Ogg Vorbis, and MP3 sources, with documented timing, trim, loop, volume, and fades. Audio mixing uses the native encoder; `--encoder ffmpeg` rejects compositions containing `<audio>`. Keep the checked audio peak at or below 1.0 to avoid clamping.

For slow motion, fast forward, or speed ramps on `<video>` or `<audio>`, use `data-hf-speed="0.5"` (0.05 to 16) or `data-hf-speed-ramp='[{"at":0,"speed":1},{"at":1,"speed":0.25,"easing":"ease-in-out-cubic"}]'` (rates 0 to 16, `at` from the clip start, strictly increasing; at least one keyframe must have a nonzero speed, so an all-zero ramp is rejected even with `data-duration`; a ramp ending at 0 freezes and needs `data-duration`). `data-media-start` and `data-hf-trim-start` are source-time offsets (2 skips two source seconds at any speed); `data-duration` and fades are clip time, never the `playbackRate` property. A video's sound follows its picture; pitch follows the rate. Smooth slow motion needs a high-frame-rate source because frames repeat rather than interpolate. Check `videos[].speed`/`mediaEnd` and `audio.clips[].mediaSeconds`, then snapshot the start, the slowest point, and the end of the ramp. See `examples/speed.html`.

For a mix with voice and music, put the effects on the clips: `data-hf-eq="voice"` on the voice, and on the music `data-hf-eq="music-bed"` plus `data-hf-duck="<voice id>"` (12 dB by default; tune `data-hf-duck-amount`, `-threshold` in dBFS, and the look-ahead `-attack`, `-hold`, and `-release` in seconds). Automate level changes with `data-hf-volume-ramp='[{"at":0,"volume":1},{"at":7,"volume":1},{"at":8,"db":-60,"easing":"ease-in-cubic"}]'` (keyframes in clip seconds, `volume` 0 to 8 or `db`, multiplied with `data-volume` and the fades). EQ JSON bands are `{type, freq, gain, q}` with types `highpass`, `lowpass`, `bandpass`, `notch`, `peak`, `lowshelf`, `highshelf`; `gain` (dB) is required on `peak`, `lowshelf`, and `highshelf`, and must be omitted on `highpass`, `lowpass`, `bandpass`, and `notch`. Each trigger needs an `id`. In the `check` report, compare `audio.clips[].effects.duck.regions` with the speech times, read `eq.responseDb`, and keep `audio.peak` at or below 1.0. See `examples/audio-effects.html`.

## Source of truth

For the exact supported gradients, shadows, text layout, clipping, CSS properties, motion options, and errors, consult the repository's `AGENTS.md` and `limits.md`.