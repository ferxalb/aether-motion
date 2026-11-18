---
name: aether-motion
description: Create and edit deterministic HTML motion graphics and short videos with AETHER Motion. Use when building a composition, sequencing clips, styling scenes, or adding supported native motion.
---

# AETHER Motion compositions

Use this skill for compositions rendered by the `aether-motion` native renderer. AETHER Motion implements a documented subset of the HyperFrames HTML format; the upstream HyperFrames project supports features this renderer does not. Do not assume browser, JavaScript, or upstream CLI features work here.

## Start from the project contract

- In an AETHER Motion checkout, read `AGENTS.md` and the closest example before writing markup. They are the source of truth for the current supported contract. Read `README.md` for installation and command examples.
- If the AETHER Motion MCP server is available, call `aether_guide` before writing. Use `aether_write`, `aether_block`, `aether_check`, `aether_snapshot`, and `aether_render` when those tools fit the task.
- For a standalone request, use the compact rules in [the composition contract](references/composition-contract.md). Check `aether-motion --help` when an installed binary's options differ.
- If the request explicitly targets upstream HyperFrames, use its own tools and documentation instead of substituting AETHER Motion.

## Build the composition

1. Translate the brief into a clear sequence of scenes, choose the requested aspect ratio and duration, and identify the main visual change in each scene.
2. Define root composition metadata and timed `.clip` elements with stable IDs, explicit starts, durations, and track indices. Use clip IDs plus offsets to align transitions when appropriate.
3. Keep every visible result a function of composition time. Prefer `data-hf-keyframes`, `data-hf-camera`, `data-hf-split`, `data-hf-reveal`, `data-hf-counter`, and `data-hf-transition` for the supported motion they describe. Use CSS animation only within the documented subset.
4. When the same video must be produced with different text, colors, numbers, or assets, declare composition variables and bind them (`var(--id)`, `data-var-text`, `data-var-src`, `data-var-if`) instead of copying the file; see the composition contract.
5. Prefer an existing AETHER Motion block or theme when one fits. If the companion `aether-motion-blocks` skill is installed, use it for discovery and parameterized markup; otherwise run `aether-motion block list`.
6. Route SVG-heavy artwork through `aether-motion-svg` when available. For an explicit migration from an existing HyperFrames or Remotion project, use `aether-motion-import` and its source-specific reference.
7. Use relative paths for local supported images (including static SVG), audio, and H.264 MP4 `<video>` clips (a source with B-frames is rejected; its `error` gives the exact `ffmpeg` re-encode command). Do not introduce network fetches, wall-clock time, unseeded randomness, scripts, SVG animation or text, remote or non-H.264 video, canvas, or unsupported CSS. If a requested effect is outside the renderer's contract, explain the limit and offer a supported representation; never silently approximate it as if it were equivalent.

## Validate and inspect

After structural edits, run `aether-motion check <file.html>` and fix errors using the reported `fix` guidance until `ok` is true; when `fix` only points to the contract, the `error` itself names the exact problem and remedy (for example the re-encode command for a video with B-frames). Then inspect representative frames, especially the start, midpoint, and end of a transition. Use `aether_snapshot` at absolute times when MCP is available; otherwise render PNG frames with the CLI's `--frame N` option. The companion `aether-motion-cli` skill has the full export workflow when installed.

Render an MP4 when the user requests a file or delivery. Preserve the requested dimensions and frame rate, and inspect the produced frame or video before describing it as complete. For 16:9 and 9:16 outputs, select `--size 1080p` or `--size 4k` and an explicit `--fps` of 30 or 60 when the request does not specify them.

## Keep the result maintainable

Keep composition markup and styles readable, use descriptive clip IDs, and keep assets local. Make the smallest change that satisfies the brief. When a change adds or moves source modules, follow the repository's architecture and file-documentation rules; a composition-only edit does not need code changes.
