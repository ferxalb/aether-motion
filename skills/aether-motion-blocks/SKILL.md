---
name: aether-motion-blocks
description: Find and use AETHER Motion's ready-made HTML blocks and themes for titles, cards, counters, calls to action, backgrounds, and controllable 2D and 3D human characters. Use before hand-building a visual or a figure that may already exist in the local block catalog.
---

# AETHER Motion blocks and themes

Use the local AETHER Motion catalog before hand-authoring a named visual. Its blocks are editable HTML templates that fit this renderer's supported contract. Do not assume the much larger upstream HyperFrames registry is available locally.

## Discover and generate

- With the CLI, run `aether-motion block list` to see available blocks, parameters, and defaults.
- With MCP, call `aether_block` without a name for the catalog, or pass `name` and `params` to generate a block.
- Generate a block with `aether-motion block <name> --set key=value`. For example: `aether-motion block hero-title --set title="Launch day" --set id=intro`.
- Copy the generated HTML into the composition and inspect its parameters before changing the markup. Give every block instance a unique `id` so its classes and keyframes do not collide.

## Fit the block into the scene

Blocks target 1920×1080, inherit timing from the clip that contains them, and read colors from the supported CSS custom properties. Add a `theme-midnight`, `theme-sunrise`, or `theme-paper` block when a shared palette helps; set the composition root background to `var(--bg)` so it follows the theme.

The local catalog includes `hero-title`, `feature-card`, `stat-card`, `counter-card`, `cta`, `glow`, the generated characters `human-2d` and `human-3d`, and the three themes. Treat the catalog output as authoritative for actual parameters and defaults. Adjust the generated HTML for the requested message and brand while preserving the block's unique identifiers and supported features.

Check the block against the shot's palette, lighting, hierarchy, and timing. Keep accents and glows local so they preserve legibility. Inspect the block at its strongest motion or effect and adapt it to the output aspect ratio before reuse.

## Characters: `human-2d` and `human-3d`

Use these for any person in a scene instead of drawing limbs by hand. They are generated: the block computes the pose and kinematics, so regenerate the block to change motion, and edit only colors in the printed HTML. Limbs are rigid segments joined on shared circles (spheres in 3D), so they never stretch or kink.

- Pick a looping `motion` (`idle`, `walk`, `run`, `wave`, `cheer`, `jump`, `sit`, `point`, or `none`), or chain motions with `sequence='[{"motion":"walk","duration":2.2},{"motion":"wave","duration":3,"blend":0.6}]'`, or key joint angles with `timeline='[{"at":0},{"at":1,"r-shoulder":150,"easing":"smooth"}]'`.
- Offset any joint with its channel name as a parameter (`r-elbow=40`, `head-side=8`, `l-hip=20`). Angles are anatomical and do not depend on facing: positive shoulder and hip values swing a limb forward, `-side` values raise it sideways, and positive knee values bend the knee.
- For walking, use `travel=auto`, which moves the figure as its feet push so a planted foot never slides. A walk that hands over to another motion in a `sequence` stops on a planted foot. Keep `ground=lock`.
- 2D: set `view=side|front`, `facing=right|left`, `x` and `y` (the ground point), and `size` (height in px). Paste the block inside a clip. To move the whole figure, wrap it and animate the wrapper, because its own elements already animate their transforms.
- 3D: the default `scene=full` is a complete `data-hf-3d` clip with lights, a floor, and a camera that follows the figure (`camera-angle`, `distance`, `orbit`). Paste it at the composition root. Use `scene=figure` to drop only the figure into your own viewport. `turn=90` makes the figure face screen right.
- Look at key frames: the start, each motion change, a stop, and the end. Keep raised hands clear of titles.

The full reference is `docs/characters.md` in an AETHER Motion checkout. `examples/characters-2d.html` and `examples/characters-3d.html` show directed scenes.

## Check the result

After inserting or editing markup, run `aether-motion check <file.html>` and inspect a snapshot. If no catalog item fits, report that and build the visual from supported primitives under the AETHER Motion composition contract.
