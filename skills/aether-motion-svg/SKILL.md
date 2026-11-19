---
name: aether-motion-svg
description: Create, adapt, and place static SVG artwork in AETHER Motion compositions. Use for vector logos, icons, diagrams, or illustrations; not for animated SVG or browser-only SVG features.
---

# Static SVG for AETHER Motion

Use SVG when vector artwork will improve an AETHER Motion composition. This renderer rasterizes supported SVG content deterministically at the element's target size; it does not run a browser SVG engine.

## Author supported artwork

- Prefer a local `.svg` asset referenced by `<img>` for reusable artwork. Use inline `<svg>` when the markup belongs only to one composition. Set a useful `viewBox`, and give the rendered HTML image or SVG element explicit width and height with supported styles.
- Stick to static shapes, paths, fills, strokes, gradients, clip paths, masks, and filters. Use only local `#fragment` references within the SVG. Keep the SVG source below 8 MiB.
- Do not use SVG `<text>`, scripts, SMIL animation, `<foreignObject>`, event handlers, or external resource URLs. Put readable copy in ordinary HTML text beside the artwork; if text must be part of the vector, convert it to paths before importing.
- Treat each `<img>` or inline `<svg>` as one visual layer. Apply timing and motion to that outer element with supported `data-hf-*` attributes. If parts need independent timing, split them into separate SVG assets or HTML elements rather than animating paths inside one SVG.
- For sizing, use a local example such as `examples/svg.html`, then account for `object-fit` and the final composition size. Avoid building SVGs at one fixed screen size when a `viewBox` can preserve their proportions.
- Under an animated camera zoom or scale, `object-fit: fill` SVG plates are resampled from shared zoom-ladder rasters (at most about 9% above the drawn size) instead of being rasterized every frame; frames at rest stay exact. Keep plates at `fill` for fast zooms, and inspect zooming frames at the export size. A plate whose fit exceeds 64 MiB is rasterized for its visible part every frame, which is slow, so keep backdrops that never need to zoom outside the camera.

## Validate the composition

Run `aether-motion check <file.html>` after inserting or changing SVG markup. Fix every reported issue, then inspect a PNG frame or use `aether_snapshot` at the intended output size. SVG validity alone does not guarantee the correct crop, scale, or contrast in the scene.
