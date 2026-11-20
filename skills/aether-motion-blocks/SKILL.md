---
name: aether-motion-blocks
description: Find and use AETHER Motion's ready-made HTML blocks and themes for titles, cards, counters, calls to action, and backgrounds. Use before hand-building a visual that may already exist in the local block catalog.
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

The local catalog includes `hero-title`, `feature-card`, `stat-card`, `counter-card`, `cta`, `glow`, and the three themes. Treat the catalog output as authoritative for actual parameters and defaults. Adjust the generated HTML for the requested message and brand while preserving the block's unique identifiers and supported features.

Check the block against the shot's palette, lighting, hierarchy, and timing. Keep accents and glows local so they preserve legibility. Inspect the block at its strongest motion or effect and adapt it to the output aspect ratio before reuse.

After inserting or editing markup, run `aether-motion check <file.html>` and inspect a snapshot. If no catalog item fits, report that and build the visual from supported primitives under the AETHER Motion composition contract.
