---
name: aether-motion-cli
description: Validate, inspect, snapshot, and render AETHER Motion HTML compositions with the native CLI or MCP tools. Use for render workflows, output sizing, and diagnosing check failures.
---

# AETHER Motion CLI and rendering

Use this skill when the task is primarily to validate a composition, inspect frames, export an image or MP4, or operate AETHER Motion through MCP. The companion `aether-motion` skill covers composition design and editing when installed.

## Prefer the connected MCP tools when available

Call `aether_guide` to retrieve the active contract. Use `aether_check` to validate, `aether_snapshot` to inspect an absolute composition time, and `aether_render` to export. `aether_snapshot` accepts `t` in seconds with `0 <= t < duration`; use it to inspect the beginning, middle, and end of motion windows. Use `aether_write` or `aether_block` only when those actions are needed for the user's request.

## Project commands

Inside a project created by `aether-motion init`, `aether.json` supplies the entry file, fps, size, and output folder. `aether-motion check` (JSON when piped), `aether-motion preview --at 2.5` (writes `.aether/preview.png`), and `aether-motion render` (writes `out/<name>.mp4`) need no arguments. `aether-motion add <block> --set key=value` appends a block, `aether-motion captions <transcript> --to json|srt|vtt` converts and regroups an existing transcript (it does not transcribe audio), and `aether-motion doctor` diagnoses skills, MCP config, and the composition.

## CLI workflow

1. Validate with `aether-motion check <composition.html>`. Read the JSON result, fix each reported issue, and rerun until `ok` is true. For 9:16 output, read `layout.findings` too: `outside_safe_zone` means text leaves the TikTok, Reels, or Shorts safe areas (10% margins on other frames); move it, or add `data-layout-allow-safe-zone` if intentional. `--caption-zone "x0=0;y0=0.7;x1=1;y1=1"` flags other text in the caption band (`caption_zone_collision`). Safe-zone findings are warnings and never fail the exit code.
2. Inspect a still with `aether-motion render <composition.html> --output frame.png --frame N`. Choose frame `N` from the target time and output FPS. For time-based snapshots or multiple moments, use MCP when available.
3. Export video with `aether-motion render <composition.html> --output video.mp4 --fps 30`. Set `--size 1080p` or `--size 4k` for 16:9 or 9:16 outputs; these map to the matching landscape or portrait dimensions. `--size` and `--scale` are alternatives. The native encoder is the default.
4. Check that the output exists and is non-empty, then inspect the result before reporting completion.

## Operational constraints

- The renderer defaults to low process priority and a limited thread count. Preserve those defaults for ordinary exports; use `--threads` when the user requests a different tradeoff.
- Native MP4 output supports the documented audio formats and encodes AAC. `--encoder ffmpeg` rejects compositions containing `<audio>`.
- `aether-motion inspect <composition.html>` can provide parsed composition details. Use it when timing or resolved structure is unclear.
- For reproducible output, state the selected size and FPS when they matter. A PNG snapshot is a single frame, not an MP4 export.
- Do not claim visual correctness from a successful check alone; inspect the rendered image or video.
