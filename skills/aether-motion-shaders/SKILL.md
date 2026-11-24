---
name: aether-motion-shaders
description: Write native procedural shaders (data-hf-shader) for AETHER Motion compositions, covering animated backgrounds, noise fields, aurora, liquid gradients, grids, starfields, grain, dissolves, rings, and glows. Use when a scene needs generative or organic imagery that boxes, gradients, SVG, or video would not give, or before reaching for a pre-rendered texture video.
---

# Procedural shaders in AETHER Motion

`data-hf-shader` fills an element's padding box with a per-pixel expression program, evaluated natively and deterministically. Start from the shader blocks before writing a program from scratch: `aether-motion block aurora`, `liquid-gradient`, `tech-grid`, `lower-third`, and `intro-scene`.

## Author a shader

- Put `data-hf-shader="..."` on a sized non-root `<div>` clip or child. The element cannot have its own text; children draw on top of it. Every other box feature (opacity, keyframes, transforms, `border-radius` with `overflow: hidden`, masks, bloom, transitions) applies.
- A program is `let name = expr;` bindings followed by one final color: a gray level, `rgb(...)`, or `rgba(...)` (straight alpha). Values have 1 to 4 components, scalars broadcast, and swizzles read components (`uv.yx`, `c.rgb`). Colors are `#rgb`, `#rrggbb`, or `#rrggbbaa`.
- Inputs: `uv` (0..1 over the box), `px` (CSS px), `size`, `aspect`, `t` (clip seconds), `pi`, and `tau`. Multiply `uv.x` by `aspect` for round shapes.
- Functions: GLSL-style math (`mix`, `smoothstep`, `clamp`, `fract`, `mod`, `step`, `pow`, `atan`, `length`, `dot`, `normalize`), shaping (`remap`, `saturate`, `tile`, `rotate`, `polar`), noise (`noise`, `fbm(p, octaves)`, `ridge`, `voronoi`, `hash`), color (`hsv`, `palette`, `luma`, `over`), and finish (`grain`, `vignette`). Make noise evolve with a third coordinate, `noise(vec3(uv * 4.0, t * 0.2))`, rather than by sliding it.
- Declare inputs with `data-hf-shader-uniforms='{"speed": 0.4, "tint": "var(--accent, #4be0b8)", "center": [0.5, 0.4]}'`. Colors become rgb in 0..1, and `var()` lets themes recolor the shader. Animate them with `data-hf-shader-keyframes='[{"at":0,"progress":0},{"at":1.2,"progress":1,"easing":"ease-in-out-cubic"}]'` for dissolves, wipes, and draw-on rings.
- Errors from `check` name the line, column, function, and component counts. Fix them and run `check` again.

## Keep it fast

- Set `data-hf-shader-resolution="0.5"` for most backgrounds and `"0.25"` for very smooth fields. Keep `1` for sharp lines, small text-like detail, and grain that must stay crisp.
- Noise dominates the cost: one 3D `noise` is about 30 operations, and an `fbm` multiplies that by its octave count. Use 3 to 5 octaves, and bind a result once with `let` and reuse it.
- A shader without `t` or keyframed uniforms is evaluated once and reused while its box stays in place.
- Measure with `aether-motion bench <file> --frame N --count 15 --threads 1`, and read `shader-eval` and `shader-blend` with `AETHER_PROFILE=1`.

## Direct the look

Shaders follow the `aether-motion-cinematic` standard. Keep backgrounds lower in contrast and saturation than the subject, so titles stay legible (`vignette`, a darker `mix` toward the edges). Keep accents local, and pair a bright shader edge with `data-hf-bloom` instead of brightening everything. Time keyframed uniforms to the shot (a dissolve that lands before the title, a ring that closes on the beat). Render frames at the start, the peak, and the end of each shader's motion and look at them. `check` alone does not show whether a field reads well.

The full reference is `docs/shaders.md` in an AETHER Motion checkout, and `examples/shader-showcase.html` shows the blocks in a directed sequence.
