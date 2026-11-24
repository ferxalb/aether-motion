# Characters: `human-2d` and `human-3d`

Two generated blocks give a composition a controllable human figure:

- **`human-2d`**: a side-view or front-view figure drawn with inline SVG and animated with CSS
  `@keyframes`. It works in any composition, at any size, with every 2D effect.
- **`human-3d`**: the same skeleton built from smooth meshes inside a native
  [`data-hf-3d`](3d.md) scene. It prints either a whole scene (viewport, lights, floor, and a
  following camera) or only the figure's node tree for a viewport you already have.

Both are computed, not templated. The block generator solves the pose, places the feet, and writes
plain HTML that you can paste, check, and render like any other markup. The render is
deterministic because the output is static markup.

```bash
aether-motion block human-2d --set motion=walk --set travel=auto --set x=300 --set y=900
aether-motion block human-3d --set motion=wave --set orbit=60 --set duration=6
```

Over MCP, call `aether_block` with `name` and `params`, for example
`{"name": "human-2d", "params": {"motion": "run", "facing": "left"}}`.

See [Characters in 2D](../examples/characters-2d.html) and
[Characters in 3D](../examples/characters-3d.html).

## Why the limbs never deform

Every body part is a rigid segment, and each joint is a circle in 2D or a sphere in 3D. A limb
segment is the exact tangent hull of the circles at its two joints: two circles joined by their
outer tangents in 2D, and spheres joined by a tube on their common tangent cone in 3D. The next
segment starts on the same circle. Elbows and knees therefore stay round at every angle and never
stretch, kink, or open a gap. Segments only move and turn. They never scale or skew.

In 2D, the generator computes the forward kinematics of the skeleton. It writes one small `<svg>`
per segment, moved and turned to the segment's world frame at each sample. Samples are refined until
no joint turns more than 2 degrees between keyframes. Linear interpolation between keyframes then
stays on the arc each joint travels, with an error under 0.1 px for a 300 px limb. Keyframes that a
straight line reproduces within 0.12 px or 0.12 degrees are dropped. In 3D, joints are nested
`<hf-node>` elements, so the renderer composes the rotations exactly.

## Parameters

Shared by both blocks:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `motion` | `idle` | A looping preset: `none`, `idle`, `walk`, `run`, `wave`, `cheer`, `jump`, `sit`, or `point`. |
| `cycle` | `auto` | Seconds per loop. `auto` uses the preset's own (walk 1.1, run 0.72, jump 1.8). |
| `phase` | `0` | Where in the loop the motion starts (0 to 1). |
| `delay` | `0` | Seconds after the clip start before the motion starts. The first pose holds until then. |
| `duration` | `6` | Clip length in seconds. Used for travel and, in 3D, for the length of the keyframes. |
| `sequence` | | A JSON list of motions to play one after another (see below). Replaces `motion`. |
| `timeline` | | A JSON list of poses at times (see below). Replaces `motion`. |
| `loop` | `false` | Repeat a `sequence` or `timeline`. |
| `travel` | `0` | Forward speed in px per second, or `auto` to follow the feet (see below). |
| `ground` | `lock` | `lock` keeps the lowest sole point on the ground. `free` leaves the hips at rest height. |
| `size` | `600` | Figure height in px (scene units in 3D). |
| `skin`, `hair`, `shirt`, `pants`, `shoes` | | Colors. 2D needs color values. 3D also takes `var()`. |
| `sleeves` | `short` | `none`, `short` (shirt-colored upper arm), or `long`. |
| *pose channels* | `0` | A static offset for each joint, added to every pose (see below). |

Only in `human-2d`:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `id` | `human` | Unique id. It prefixes every element id and keyframe name. |
| `view` | `side` | `side` (profile) or `front`. |
| `facing` | `right` | Direction of a side view: `right` or `left`. |
| `x`, `y` | `960`, `900` | Ground point under the figure, in composition px. |
| `shadow` | `on` | A soft contact shadow on the ground. |

Only in `human-3d`:

| Parameter | Default | Meaning |
| --- | --- | --- |
| `id` | `human3d` | Unique id. It prefixes every node id. |
| `scene` | `full` | `full` prints a `data-hf-3d` clip with lights, floor, grid, contact shadow, and camera. `figure` prints only `<hf-node id="{id}-track">` with the feet at y = 0. |
| `x`, `z` | `0` | Ground position in scene px. |
| `turn` | `0` | Heading in degrees, in the CSS `rotateY` sense: 0 faces the camera and 90 faces +x (screen right). |
| `start`, `track` | `0`, `1` | `data-start` and `data-track-index` of the full scene's clip. |
| `left`, `top`, `width`, `height` | `0`, `0`, `1920`, `1080` | The full scene's viewport box. |
| `antialias` | `3` | `data-hf-3d-antialias` of the full scene (1 to 4). |
| `camera-angle` | `25` | Camera position around the figure, in degrees (positive moves the camera to screen right). |
| `camera-height`, `distance` | `330`, `1500` | Eye height and distance, scaled with `size`. The camera aims at the figure's waist and follows it. |
| `fov` | `35` | Vertical field of view in degrees. |
| `orbit` | `0` | Degrees the camera sweeps around the figure over `duration`, with `smooth` easing. |
| `floor`, `grid` | | Floor and grid-line colors. A `#rrggbb` floor also shades the contact shadow. |

## Pose channels

Angles are anatomical degrees and do not depend on view or facing:

| Channel | Positive values |
| --- | --- |
| `body-x`, `body-y` | Move the whole body forward or down, in units of a 600-unit figure (px at `size=600`). |
| `body` | Lean the whole body forward around the hips. |
| `torso`, `torso-side`, `torso-twist` | Bend the spine forward, lean to the figure's right, or turn to its right (twist: 3D only). |
| `head`, `head-side`, `head-turn` | Nod down, tilt to the right, or turn to the right (turn: 3D only). |
| `r-shoulder`, `l-shoulder` | Swing the arm forward and up (flexion; 180 is straight overhead). |
| `r-shoulder-side`, `l-shoulder-side` | Raise the arm sideways (abduction; 90 is horizontal). |
| `r-elbow`, `l-elbow` | Bend the elbow. |
| `r-wrist`, `l-wrist` | Bend the hand forward. |
| `r-hip`, `l-hip` | Swing the thigh forward (90 is horizontal, as when sitting). |
| `r-hip-side`, `l-hip-side` | Spread the legs. |
| `r-knee`, `l-knee` | Bend the knee, moving the shin back. |
| `r-ankle`, `l-ankle` | Lift the toes. Negative values point them. |

Right and left are the figure's own. A side view facing right shows the right side nearest the
viewer, and the far limbs are drawn darker, behind the body. The projection decides which channels a
2D view shows:

- **Side view**: `body`, `torso`, `head`, and the flexion of shoulders, elbows, wrists, hips, knees,
  and ankles.
- **Front view**: `torso-side`, `head-side`, `shoulder-side`, `hip-side`, and the elbows and wrists,
  which bend in the picture plane toward the same side as a raised arm. Knees and ankles do not show
  in a front view.
- **3D**: every channel.

Presets are shaped for the view. For example, `wave` raises the arm forward in a side view and
sideways in a front view or in 3D.

## Motions in sequence

`sequence` chains motions. Each step needs `motion` and `duration` (seconds). Optional fields are
`cycle`, `phase`, and `blend`. `blend` is the time to cross into the step: 0, or from 0.01 s up to the
step's duration. It defaults to 0.4 s (or the step's duration if shorter), and to 0 for the first
step. A sequence lasts at most 600 s. Any pose channel in a step is an offset for that step only:

```bash
aether-motion block human-2d --set travel=auto --set duration=6 --set sequence='[
  {"motion": "walk", "duration": 2.2},
  {"motion": "wave", "duration": 3.8, "blend": 0.6, "head-side": 6}
]'
```

When a walk or run hands over to anything else, it ends on mid-stance: one sole is flat and
planted, and the other foot is high in its swing. The gait then holds that pose while the next motion
blends in, so the figure stops on a planted foot instead of stepping on in place. Give the gait step
a `phase` to choose the ending yourself. Two gaits in a row, such as walk into run, keep stepping
through the blend.

## Keyed poses

`timeline` keys absolute poses at times in seconds from the motion start (after `delay`):

```bash
aether-motion block human-2d --set motion=none --set timeline='[
  {"at": 0},
  {"at": 0.8, "r-shoulder": 160, "r-elbow": 30, "easing": "ease-out-cubic"},
  {"at": 1.6, "r-shoulder": 160, "r-elbow": 70, "easing": "smooth"},
  {"at": 2.4, "r-shoulder": 0, "r-elbow": 6, "easing": "ease-in-out"}
]'
```

Keys are at most 600 s and at least 1 ms apart, and the first key is at 0 or at least 1 ms after
it, so every key keeps its own keyframe. Each key holds
every channel. An omitted channel keeps the previous key's value, starting from the
rest pose, which has the arms 5 degrees from the body. The easing of a key shapes the segment that
ends there. The easings are `linear`, `ease-in`, `ease-out`, `ease-in-out`, `ease-in-cubic`,
`ease-out-cubic`, `ease-in-out-cubic`, and `smooth`, a Catmull-Rom curve through the keys that comes
to rest at the first and last keys. The last pose holds after the final key unless `loop=true`.

## Ground and travel

With `ground=lock` (the default), the hips are raised or lowered at every sample so the lowest sole
point touches the ground. The rig computes this with the same kinematics that place the drawn feet,
so crouches, sitting, and the stance of a walk all stand on the floor. A preset's own lift, such as
the flight of `jump` or `run`, is added on top.

`travel=auto` moves the figure as its feet push. The rig follows the lowest sole point and holds it
still on the ground. When another point takes over, the hold passes to it without a jump. A point
keeps its hold while it stays within 4 units of the lowest. For a looping walk or run, the result is
a constant speed plus a small periodic surge of the hips. For a `sequence` or a `timeline`, it is the
whole path, which ends where the figure stops. Planted feet do not slide. Measured on a 1080p
render, a planted sole stays within 1 px through each stance. `travel=auto` needs a side view in 2D.
A number instead is a constant speed in px per second along the facing direction. Travel needs at
least 1 ms between `delay` and `duration`. `travel=auto` follows joints that turn up to 3 degrees
between its samples, which it refines down to 10 microseconds. A keyed move faster than that is an
error, not a sliding foot.

In 3D, the camera and the contact shadow follow the figure's travel.

## Using the output

- **2D**: paste the block inside a clip of the composition. Its root is a 1 px anchor at `x`, `y`,
  scaled by `size / 600`, with the segments and shadow inside. Each segment's motion is a CSS
  animation on the segment's `<svg>`, so the rest of the clip's effects apply on top: fades,
  cameras, parallax, bloom, filters, motion blur, and transitions. The block's own elements must not
  get other `transform` animations. To move the whole figure, wrap the block in an element and
  animate that element.
- **3D full scene**: the block is a clip of its own, so paste it directly inside the composition
  root and set `start`, `duration`, and `track`. Layer 2D titles and skies above or below it.
- **3D figure**: paste `scene=figure` inside your own `data-hf-3d` viewport, at its top level or in
  a group, with your own lights and camera. The feet stand at y = 0 of the parent.
- Regenerate the block to change motion, timing, or pose. Colors in the printed HTML are plain
  attributes and CSS values, so you can edit them in place.

## Cost

These measurements used one thread at 1920x1080:

| Block | Per frame |
| --- | --- |
| Two `human-2d` figures at 600 px (walk and wave) | 52 ms |
| One `human-3d` full scene (walk, about 35,000 triangles), `antialias=3` | 173 ms |
| The same scene with `antialias=2` | 100 ms |

A 2D figure draws about 16 small rotated layers. A 3D figure uses smooth shading, which the CPU
rasterizer evaluates per sample, so `antialias=2` saves about 42% at a small cost to edges. A
looping 2D walk is about 70 KB of HTML. A 3D figure is about 110 KB, most of it mesh data.
