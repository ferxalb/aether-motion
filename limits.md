# Limits

- SVG (`<img src="x.svg">` or inline `<svg>`) is static: no `<text>`, SMIL animation, scripts, `<foreignObject>`, or external resources. Scripts of any kind remain unsupported by design (determinism); use the native `data-hf-*` motion instead.
- Descendants of a rotated element are clipped to its box plus its shadow reach. This is a deliberate, documented limitation.
- There is no skew, 3D, percent translation, or other transform origin; these return an explicit error.
- `filter` takes an optional `blur()`, then color functions, then an optional `drop-shadow()`, in that order (no `url()`). Blur and drop-shadow are not allowed on the root, and the root accepts no `opacity()`; children that stick out of the element by more than the blur and drop-shadow reach plus the element's shadow reach are cut. Filter amounts animate on non-root elements only; `drop-shadow()` and `opacity()` do not animate, and CSS `@keyframes` filter lists interpolate per function rather than switching discretely when their functions differ.
- `mix-blend-mode` supports the separable modes and `plus-lighter` only (no `hue`, `saturation`, `color`, or `luminosity`), never on the root, and never inside `overflow: hidden`, rotation, reveals, wipes, filters, or another blended element.
- Motion blur (`data-motion-blur`) is set for the whole composition on the root (no per-element opt-out), costs about one full draw per sample, and snapshots assume 30 fps for the shutter window.
- `backdrop-filter` supports only `blur()`, and inside `overflow: hidden` or rotated elements it blurs that element's layer, not the whole scene.
- Scene transitions cover crossfade, push, cover, wipe, and zoom. Shader transitions are unsupported. Wipes and `data-hf-reveal` clip to the element's box, so shadows outside it are cut while the reveal is partial, and they cannot be combined with rotate.
- `data-hf-split` handles one line of plain text (up to 400 pieces) and places letters without kerning.
- `box-shadow` takes up to 8 layers, outer or `inset`; `text-shadow` takes one non-inset shadow.
- Audio: WAV, FLAC, Ogg Vorbis, and MP3 sources, mono or stereo only; AAC-LC output at 48 kHz stereo (no surround, no HE-AAC). Fades are linear, loops cut hard, volume is not animatable, and `--encoder ffmpeg` cannot carry audio. 16-bit quantization has no dither. The mix is capped at about 22 minutes. `<audio>` inside a clip starts with it but is not cut when the clip ends.
