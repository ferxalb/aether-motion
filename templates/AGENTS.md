# Video project (AETHER Motion)

HTML compositions rendered to MP4 by `aether-motion`. Output is deterministic: a frame is a function of time.

## Work loop

1. Edit `index.html` (entry set in `aether.json`).
2. `aether-motion check` until it reports `ok`; the error includes a `fix`.
3. `aether-motion preview --at 2.5` to look at a moment (writes `.aether/preview.png`).
4. `aether-motion render` writes `out/<name>.mp4` at the `standard` quality preset; pass `--quality draft` (or set `"quality": "draft"` in `aether.json`) for smaller review exports.

Agents: the `aether-motion` MCP server is configured in `.mcp.json` (tools `aether_guide`, `aether_check`,
`aether_snapshot`, `aether_render`, `aether_block`, `aether_write`). Skills live in `.claude/skills/` and
`.agents/skills/`. Call `aether_guide` for the exact supported HTML/CSS contract; unsupported features fail
with an explicit error.

Ready-made scenes: `aether-motion add` lists blocks, `aether-motion add <block> --set key=value` appends one.
For a person, use the character blocks: `aether-motion add human-2d --set motion=walk --set travel=auto`
(or `human-3d`). They have motion presets, `sequence`, `timeline`, and per-joint offsets such as `r-elbow=40`.
