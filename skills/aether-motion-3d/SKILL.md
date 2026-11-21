---
name: aether-motion-3d
description: Build native low-poly 3D shots in AETHER Motion with data-hf-3d (meshes, flat Lambert or unlit materials, ambient and directional lights, an animated 3D camera) and composite them with 2D layers. Use when a composition needs real geometry, a camera that orbits or cranes through objects, or depth that flat 2.5D planes cannot give.
---

# Native 3D scenes in AETHER Motion

Use this skill for `data-hf-3d` viewports. The authoritative contract is the `data-hf-3d` entry in `AGENTS.md` and `docs/3d.md` in an AETHER Motion checkout (`aether_guide` over MCP). The `aether-motion` skill covers compositions in general, and `aether-motion-cinematic` sets the directing standard. Follow both.

## Choose 3D, 2.5D, or 2D

- **`data-hf-3d`**: solid objects seen from changing angles (orbits, cranes, turntables), intersecting geometry, and lighting that follows form. The scene is flat-shaded low-poly by design.
- **`data-hf-space`**: illustrated plates, photos, text, or SVG placed in depth. Prefer it when the artwork is 2D.
- Combine them: a 3D viewport is an ordinary element, so it can sit between 2D sky and title layers, inside a 2D camera, or as a plane of a 2.5D space.

## Build the scene

1. Put one sized clip in the composition: `<div class="clip" data-hf-3d data-hf-3d-antialias="2|3" style="position:absolute; left:0; top:0; width:1920px; height:1080px">`. Leave its background transparent when 2D layers (sky, gradients, glow) draw behind it.
2. Remember the axes: origin at the box center, x right, **y down (up is -y)**, z toward the viewer. A ground plane is `<hf-mesh shape="plane" size="4000 4000">` at `y = 0`. Things standing on it have negative y: a box of height `h` sits at `position="x -h/2 z"`.
3. Group with `<hf-node>` for pivots and orbits. Animate groups, not every mesh. Rotation is `rotation="rotateX rotateY rotate"` in CSS senses, so `rotateY` turns a turntable.
4. Meshes are `plane`, `box`, `pyramid`, `cone`, `cylinder`, `sphere` (icosphere `detail` 0 to 4), or `custom` (`vertices` and `indices`, front faces wound so `(b-a)x(c-a)` points outward). Keep the total triangle count modest: the scene limit is 100,000, and cost grows with covered pixels, not triangles.
5. Light deliberately: one `ambient` (sky color, about 0.4 to 0.6) plus one warm `directional` key (`direction` is the way the light travels) and optionally a cool fill. Match the key to any visible 2D sun or light source. Use `material="unlit"` for emissive-looking parts (lamps, screens, beams) and pair them with `data-hf-bloom` on the viewport (threshold about 0.8) for glow.
6. Close every `<hf-*>` tag explicitly. Unknown attributes, wrong shapes, and stray text are errors with a `fix`.

## Direct the camera

- Use one `<hf-camera>` and move it with `data-hf-keyframes` (`x`, `y`, `z`, `yaw`, `pitch`, `roll`, `zoom`) and `"easing":"smooth"`, so velocity stays continuous across keys.
- `target="#node"` or `target="x y z"` keeps the subject framed with a level horizon while the camera cranes or orbits. Offset the composition with `yaw` and `pitch` (for example `yaw: -12` moves the subject right of center to leave room for a title).
- For a perfect circular orbit, put the camera inside an `<hf-node>` at the subject and keyframe that node's `rotateY`.
- Aim the subject's middle, not its base, so the horizon stays in frame. When 2D layers must track the 3D world (a horizon band or a sun plate), compute their screen positions from the same camera math and keyframe them. `examples/lighthouse-isle-3d.html` shows this.
- Keep geometry beyond `near` (default 10 px) and within `far` (default 50,000 px), and keep big ground planes large enough that their far edge never shows.

## Verify

1. Run `aether-motion check`. Read `scenes3d` (triangles, meshes, lights, camera, `animated`, `backend`).
2. Render stills at the start, the middle and end of every camera move, the moments where the camera passes close to geometry, and the final hold: `--output 'f-{frame}.png' --frames ...` at delivery size. Look for clipped subjects, a tilted or exposed horizon, geometry crossing the near plane, z-fighting on coplanar faces (offset them by a pixel or two), missing faces (wrong winding or `cull`), and light that disagrees with the 2D plates.
3. Export with the CPU backend (the default) for deterministic, byte-identical results. `--3d-backend gpu` exists only in builds with `--features gpu`. Its output matches the CPU within a small edge tolerance, and it falls back to the CPU when no GPU works.
4. Budget time: at 1080p a full-frame scene costs roughly 27, 37, 61, and 95 ms per frame at `antialias` 1, 2, 3, and 4 (single thread), and effect layers such as bloom draw it twice (the second draw is replayed from a memo).

## Do not

Promise textures, PBR, smooth shading, transparency per material, point or spot lights, shadows, fog, glTF or other model files, skeletal animation, physics, or 3D text. These are not supported. Offer a flat-shaded, unlit, or 2D-overlay alternative instead, and say so.
