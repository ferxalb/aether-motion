# Limits

- Descendants of a rotated element are clipped to its box plus its shadow reach. This is a deliberate, documented limitation.
- There is no skew, 3D, percent translation, or other transform origin; these return an explicit error.
- `backdrop-filter` supports only `blur()`, and inside `overflow: hidden` or rotated elements it blurs that element's layer, not the whole scene.
- Scene transitions cover crossfade, push, cover, wipe, and zoom. Shader transitions are unsupported. Wipes and `data-hf-reveal` clip to the element's box, so shadows outside it are cut while the reveal is partial, and they cannot be combined with rotate.
- `data-hf-split` handles one line of plain text (up to 400 pieces) and places letters without kerning.
- `box-shadow` takes up to 8 outer shadows; `inset` is unsupported.
