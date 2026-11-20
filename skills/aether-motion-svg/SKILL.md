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
- Under an animated camera zoom or scale, `object-fit: fill` SVG plates are resampled from shared zoom-ladder rasters (at most about 9% above the drawn size, plus a pixel of rounding) instead of being rasterized every frame, as long as that ladder raster stays within the 64 MiB per-image limit; frames at rest stay exact. A plate near the limit can push its ladder raster over it even at a small zoom, and it is then rasterized again on every frame whose size changes. Keep plates at `fill` for fast zooms when their ladder raster stays within the limit (check `image-resample` and `svg-raster` counts with `AETHER_PROFILE=1`), and inspect zooming frames at the export size. A plate whose fit exceeds 64 MiB is rasterized for its visible part every frame, which is slow, so keep backdrops that never need to zoom outside the camera.

## Preserve geometry and proportions

Match the element's aspect ratio to the `viewBox` when using `fill`. Use `contain` to keep the whole asset or `cover` for an intentional crop. For human silhouettes, verify joint placement, body proportions, and pose against the reference at delivery size. Use finished raster artwork when the vector cannot preserve the intended anatomy or detail. Inspect stroke joins, caps, masks, and silhouettes during the strongest zoom or motion.

## Validate the composition

Run `aether-motion check <file.html>` after inserting or changing SVG markup. Fix every reported issue, then inspect a PNG frame or use `aether_snapshot` at the intended output size. SVG validity alone does not guarantee the correct crop, scale, or contrast in the scene.
