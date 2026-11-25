# Procedural shaders

`data-hf-shader` fills an element's padding box with an image computed per pixel by a small
expression program. Use it for animated backgrounds (aurora, liquid gradients, grids, starfields),
procedural textures, dissolves and wipes with organic edges, glows, rings, and data-free
decorative motion. Shaders run natively. They need no GPU, and every pixel is a pure function of the
program, the clip time, the uniforms, and its position, so renders are identical on every machine,
thread count, and render split.

```html
<div class="clip" data-start="0" data-duration="6"
     data-hf-shader="
       let n = fbm(vec3(uv * 3.0, t * 0.2), 5);
       rgb(mix(#0b1026, #6d5dfc, smoothstep(0.3, 0.8, n)))
     "
     data-hf-shader-resolution="0.5"
     style="position: absolute; left: 0; top: 0; width: 1920px; height: 1080px"></div>
```

The element is an ordinary box. Position, size, `opacity`, keyframes, transforms, `border-radius`
with `overflow: hidden`, `clip-path`, masks, filters, bloom, blend modes, transitions, and cameras
apply to it as to any `div`. Its children are drawn on top of the shader. The element itself
cannot have text: put text in a child.

## Attributes

| Attribute | Value |
| --- | --- |
| `data-hf-shader` | The program (required). |
| `data-hf-shader-uniforms` | JSON object of named inputs: a number, an array of 2 to 4 numbers, or a CSS color string (`"#ff8800"`, `"rgb(...)"`, or `"var(--accent, #4be0b8)"` from the element's custom properties, so themes recolor shaders). Colors become `rgb` values from 0 to 1. Give an array for alpha. At most 32. |
| `data-hf-shader-keyframes` | JSON array of `{"at": seconds, "easing": ..., "<uniform>": value}` objects that animate declared uniforms over clip time. Each uniform holds its first value before its first key and its last value after its last key. `easing` (any keyframe easing, for example `ease-out-expo` or `spring(1.2, 2)`) shapes the segment that ends at that keyframe. |
| `data-hf-shader-resolution` | `1` (default), `0.5`, or `0.25`. This is the fraction of output pixels evaluated per axis. Lower values evaluate 4 or 16 times fewer points and interpolate between them bilinearly. The grid is anchored to the box, so the pattern never swims. Use `0.5` for most backgrounds and `0.25` for very smooth fields. Keep `1` for sharp lines and fine grain. |
| `data-hf-shader-dither` | `on` (default) or `off`. A 4x4 ordered dither of under half an 8-bit level hides banding in smooth gradients. |

## Language

A program is a list of `let name = expression;` bindings followed by one final expression: the
color. A later binding may reuse a name (`let c = mix(c, glow, 0.5);` reads the earlier `c`). The final value can be a gray level (1 component), `rgb` (3), or `rgba` (4, straight alpha).
Components are clamped to 0..1. `//` starts a comment.

Values have 1 to 4 components. Arithmetic (`+ - * / %`) and every function apply per component,
and a 1-component value broadcasts against a vector. Swizzles read components: `.x .y .z .w`
or `.r .g .b .a`, in any order and count (`p.yx`, `c.rgb`, `v.xxx`). Colors can be written
`#rgb`, `#rrggbb`, or `#rrggbbaa`. Division by zero gives 0, `sqrt` and `log` of non-positive values
are clamped, and `pow` of a non-positive base gives 0, so a program never produces NaN.

### Built-in values

| Name | Meaning |
| --- | --- |
| `uv` | Position in the padding box, `(0, 0)` top-left to `(1, 1)` bottom-right. |
| `px` | Position in CSS px from the padding box's top-left corner. |
| `size` | Padding box size in CSS px. |
| `aspect` | `size.x / size.y`; multiply `uv.x` by it for round shapes. |
| `t` | Seconds since the clip started. |
| `pi`, `tau` | π and 2π. |

### Functions

| Function | Result |
| --- | --- |
| `vec2`, `vec3`, `vec4`, `rgb`, `rgba` | Build a vector from components or smaller vectors. `vec3(0.5)` repeats one value. |
| `abs`, `floor`, `ceil`, `fract`, `sign`, `sqrt`, `exp`, `exp2`, `log`, `log2`, `sin`, `cos` | Per component. |
| `min`, `max`, `mod`, `pow`, `step(edge, x)`, `atan(y, x)` or `atan(v)` | Per component. `mod` follows GLSL (sign of the divisor). |
| `clamp(x, lo, hi)`, `saturate(x)`, `mix(a, b, t)`, `smoothstep(e0, e1, x)`, `remap(x, in0, in1, out0, out1)` | Interpolation and shaping. Reversed `smoothstep` edges invert the ramp. `remap` clamps. |
| `length`, `distance`, `dot`, `normalize` | Vector geometry. |
| `rotate(p, angle)` | Turns a 2D point around the origin (radians). |
| `polar(p)` | `(distance, angle in turns from 0 to 1)` of a 2D point. |
| `tile(p, n)` | `fract(p * n)`: repeats the unit square `n` times. |
| `noise(p)` | Gradient noise of a 2D or 3D point, 0..1. Use the third coordinate for time to make the pattern evolve instead of slide. |
| `fbm(p, octaves)` | Fractal sum of `noise` (2D or 3D), 0..1. `octaves` is a whole number from 1 to 8 (default 5). Each octave costs about one `noise`. |
| `ridge(p)` or `ridge(p, octaves)` | `1 - abs(2 noise - 1)`: sharp crests for mountains, lightning, and veins. |
| `voronoi(p)` | Distance to the nearest point of a jittered grid (2D), 0 at a point and about 0.5 at cell borders. |
| `hash(p)` | A random value in 0..1 per integer cell of a 1D or 2D point. Combine with `floor(px / n)` for blocks and stars. |
| `grain(amount)` | Film grain of the given amplitude that changes 24 times per second. It reads `px` and `t`. |
| `vignette(strength)` | 1 at the center, falling toward `1 - strength` in the corners. |
| `hsv(h, s, v)` | HSV to rgb. The hue is in turns. |
| `palette(t, a, b, c, d)` | Cosine palette `a + b cos(tau (c t + d))`. |
| `luma(color)` | Rec. 709 luminance. |
| `over(top, bottom)` | Source-over compositing of a straight-alpha `rgba` on an `rgb` or `rgba`. |

All math is implemented in the renderer with IEEE operations only (no system math library), so
`sin`, `exp`, and noise give the same bits on every platform.

## Recipes

```text
// Aurora ribbons
let warp = fbm(vec3(uv * 2.0, t * 0.08), 4);
let band = fbm(vec3(uv.x + warp, uv.y * 3.0, t * 0.05), 5);
rgb(#050816 + #4be0b8 * smoothstep(0.5, 0.78, band) * (1.0 - uv.y))

// Noise dissolve driven by a keyframed uniform `progress`
let f = uv.x * 0.8 + fbm(px / 40.0, 4) * 0.2;
rgba(#121a2e, smoothstep(progress, progress - 0.03, f))

// Ring that draws itself
let pol = polar((uv - 0.5) * 2.0);
let along = fract(pol.y + 0.25);
rgba(#4be0b8, smoothstep(0.04, 0.0, abs(pol.x - 0.8)) * step(along, progress))

// Starfield
vec3(step(0.997, hash(floor(px / 3.0))))
```

The blocks `aurora`, `liquid-gradient`, `tech-grid`, `lower-third`, and `intro-scene`
(`aether-motion block list`) are complete shader-based backgrounds and scenes. Start from them.

## Performance

The program is compiled once at parse time. Constants are folded, repeated subexpressions are
shared, unused bindings are dropped, and every vector operation is lowered to scalar instructions
on reused registers. The renderer evaluates 64 neighboring points per instruction in loops that
the compiler vectorizes. On x86-64 CPUs with AVX2 the loops run in an AVX2 build with identical
results (`AETHER_SIMD=off` forces the baseline build). The cost is proportional to the number
of points × instructions. Noise calls dominate: a 3D `noise` is about 30 operations, and an `fbm`
is that times its octaves.

- Use `data-hf-shader-resolution="0.5"` or `"0.25"` for smooth fields. It is the largest single saving.
- Keep octave counts low (3 to 5 for backgrounds), and reuse one `fbm` result through `let` rather than calling it twice.
- A still shader (no `t`, no keyframed uniform) is evaluated once and reused while its box keeps the
  same placement on screen. Only the blend runs on later frames. An animated shader is evaluated on
  every frame that shows it. Elements share a 64 MiB budget for kept grids.
- `AETHER_PROFILE=1` reports `shader-eval` (the program) and `shader-blend` (resampling and
  compositing) thread time.

## Limits

Programs are at most 16 KiB, lower to at most 8,192 operations, and may keep at most 1,024
values alive. A grid is at most the visible part of the box at output resolution.
Errors name the line and column, the function, and the component counts involved.
