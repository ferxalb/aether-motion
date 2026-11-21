---
name: aether-motion-cinematic
description: Direct cinematic AETHER Motion scenes that are consistent, complete, and use the engine's current capabilities. Use whenever creating, redesigning, or reviewing a narrative scene, illustrated short, character shot, camera move, or multi-shot sequence.
---

# Cinematic scene design for AETHER Motion

The rule is: design well. A scene is finished when it reads as deliberately directed, with continuous camera work, finished characters, and no visual accidents. Passing `check` is required, not sufficient. The companion `aether-motion` skill covers the composition contract; `aether-motion-cli` covers rendering.

## 1. Start from the current engine

The renderer changes often. Before designing, read the Supported contract in `AGENTS.md` and the newest examples (`git log --oneline -- examples` shows them), then choose the strongest native tool for each need:

| Need | Prefer |
| --- | --- |
| One continuous camera through a space | `data-hf-shots` (`follow`, `dolly-in`, `pan`, `reveal`) on a rig camera with `data-hf-camera-subject` |
| Depth | A `data-hf-space` with planes at `data-hf-z` depths (true perspective, tilted floors and cards, camera `z` dolly, pan, and tilt; intersecting planes are ordered exactly); `data-hf-parallax` planes for a flat rig shot. Overscan either way, and stand uprights on a floor (their base on its surface) so nothing sinks into it |
| Solid objects seen from changing angles | A `data-hf-3d` viewport (meshes, lights, a `target`ed camera with `smooth` keyframes) between 2D sky and title layers; see the `aether-motion-3d` skill |
| Characters | `data-hf-sequence` cels on a shared registration, plus a matching hold cel |
| Travel along a route | `data-hf-motion-path` |
| Joining two spaces | `data-hf-transition` (`crossfade` as a match dissolve, `blur`, `whip-*`, `dip-to-black` to end an act) |
| Light and atmosphere | `data-hf-bloom`, drifting mist and cloud plates, root exposure, rolloff, toe, vignette, grain |
| Speed | `data-hf-motion-blur` on fast subjects, `spring`/`smooth` easings |
| Sound | an ambience bed under the whole sequence, `data-hf-volume-ramp`, `data-hf-duck` |

If a newer feature does what an older scene does by hand, use the newer feature.

## 2. Plan shots before markup

Write the beat list first: what each shot shows, why the camera moves, and how it hands off to the next one. For each handoff, decide:

- **Same space:** keep one camera and move it (follow → settle → reveal), with no cut. Default to `smooth` easing so velocity is continuous across shots.
- **Different space or angle:** use a motivated transition. For a dissolve, keep both cameras moving in the same direction and place the subject and dominant shapes (horizon, moon, light source) at the same screen positions mid-dissolve so they overlap instead of doubling. Compute positions from the camera math, then confirm them on a snapshot.
- **Ending:** resolve on a held wide or a fade that the sound also resolves.

Avoid hard cuts unless the brief asks for them, axial jumps of a few percent, cameras that tilt the subject out of frame, and moves without a reason.

## 3. Finish characters and elements

- Keep one drawing standard per film: proportions, line weight, palette, light direction, and contact shadow match in every shot and at every scale.
- Walk at a speed that matches the stride (no foot sliding), decelerate into a stop, and end the cycle on the cel whose stance matches the hold cel. Give every cel, hold, and timed layer an explicit `data-duration` that covers the rest of its shot; an unset duration defaults to 3 s and the layer disappears.
- Ground the subject: contact shadow, matching rim or moon light, foreground elements that pass in front of it.
- Overscan every plane for the widest zoom and the largest pan, and keep plate aspect ratios intact (no stretched SVGs).
- Add life to still plates with slow deterministic drift (clouds, mist, shimmer, surf) so no frame looks frozen.

## 4. Verify by looking

1. `aether-motion check` on the full sequence and on each external scene; read `cameras`, `clips`, and `animations` for wrong durations or key times.
2. Render stills at the opening, mid-walk, the stop, the start, middle, and end of every camera move and transition, and the final frame. Use one process: `--output 'f-{frame}.png' --frames ...` at the export `--fps`.
3. Look for: the subject leaving the frame or vanishing, doubled shapes in dissolves, exposed plate edges, foot sliding, mismatched light or scale between shots, and dead frames.
4. Fix, resample the affected moments, then render the MP4 and update the example's README with the shot structure.
