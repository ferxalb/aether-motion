---
name: aether-motion-cli
description: Validate, inspect, snapshot, and render AETHER Motion HTML compositions with the native CLI or MCP tools. Use for render workflows, output sizing, and diagnosing check failures.
---

# AETHER Motion CLI and rendering

Use this skill when the task is primarily to validate a composition, inspect frames, export an image or MP4, or operate AETHER Motion through MCP. The companion `aether-motion` skill covers composition design and editing when installed.

## Prefer the connected MCP tools when available

Call `aether_guide` to retrieve the active contract. Use `aether_check` to validate, `aether_snapshot` to inspect an absolute composition time, and `aether_render` to export. `aether_snapshot` accepts `t` in seconds with `0 <= t < duration`; use it to inspect the beginning, middle, and end of motion windows. Use `aether_write` or `aether_block` only when those actions are needed for the user's request.

## Project commands

Inside a project created by `aether-motion init`, `aether.json` supplies the entry file, fps, size, quality, and output folder. `aether-motion check` (JSON when piped), `aether-motion preview --at 2.5` (writes `.aether/preview.png`), and `aether-motion render` (writes `out/<name>.mp4`) need no arguments. `aether-motion add <block> --set key=value` appends a block, `aether-motion captions <transcript> --to json|srt|vtt` converts and regroups an existing transcript (it does not transcribe audio), and `aether-motion doctor` diagnoses skills, MCP config, and the composition.

## CLI workflow

1. Validate with `aether-motion check <composition.html>`. Read the JSON result, fix each reported issue, and rerun until `ok` is true. For 9:16 output, read `layout.findings` too: `outside_safe_zone` means text leaves the TikTok, Reels, or Shorts safe areas (10% margins on other frames); move it, or add `data-layout-allow-safe-zone` if intentional. `--caption-zone "x0=0;y0=0.7;x1=1;y1=1"` flags other text in the caption band (`caption_zone_collision`). Safe-zone findings are warnings and never fail the exit code.
2. Inspect a still with `aether-motion render <composition.html> --output frame.png --frame N`. Choose frame `N` from the target time and output FPS. For several stills, render them from one process with `--output 'frame-{frame}.png' --frames 0,45,90` (the name must contain `{frame}`). For time-based snapshots or multiple moments, use MCP when available.
3. Export video with `aether-motion render <composition.html> --output video.mp4 --fps 30`. Set `--size 1080p` or `--size 4k` for 16:9 or 9:16 outputs; these map to the matching landscape or portrait dimensions. `--size` and `--scale` are alternatives. The native encoder is the default. `--quality draft|standard|high|max` picks the H.264 quantizer (30, 26, 22, 18); the default `standard` suits most deliveries. Higher presets cost about the same CPU and give larger files; use them only when the user asks for it. Use `--quality draft` for small review exports.
4. Check that the output exists and is non-empty, then inspect the result before reporting completion. Long exports print a progress line with an ETA on a terminal; when a script or job runner needs progress, pass `--progress json` (one JSON object per update on stderr) or `--progress off`. Through MCP, send `_meta.progressToken` with the `aether_render` call to receive `notifications/progress`.

## Templates with variables

For `<video>` or `<audio>` with `data-hf-speed` or `data-hf-speed-ramp`, read `videos[].speed` and `mediaEnd` and `audio.clips[].speed`, `end`, and `mediaSeconds` in the `check` report to confirm how much source each clip consumes, then snapshot before, inside, and after each ramp segment (`aether-motion preview --at T` or `aether_snapshot` with `t`). A ramp that ends at speed 0 without `data-duration` fails with a `fix`.

For audio effects (`data-hf-volume-ramp`, `data-hf-eq`, `data-hf-duck`), `aether-motion check` reports `audio.clips[].effects`: verify that `duck.regions` cover the voice with the intended look-ahead and release, that `duckedSeconds` and `deepestDb` match the requested amount, that `eq.responseDb` has the intended shape, and that `audio.peak` stays at or below 1.0 before rendering. Render with the native encoder (`--encoder ffmpeg` rejects audio).

When the composition declares `data-composition-variables`, pass values as a JSON object keyed by id: `--variables '{"title":"Q4","accent":"#ff7a45"}'` or `--variables-file values.json` on `check`, `render`, `preview`, `bench`, and `inspect`, `"variables"` in `aether.json` (flags override it id by id), or the `variables` argument of `aether_check`, `aether_write`, `aether_snapshot`, and `aether_render`. Omitted ids keep their defaults; unknown ids and wrong types fail with a `fix`. Read `variables` in the `check` report (`value`, `overridden`, `uses`, `unused`, `removedElements`), and run `check` and a snapshot with every value set you deliver, because values change text length, layout, and safe-zone findings. For batches, render each value file to its own output name.

## Operational constraints

- The renderer defaults to low process priority and a limited thread count. Preserve those defaults for ordinary exports; use `--threads` when the user requests a different tradeoff.
- Output is byte-identical for any `--threads` value, any distributed split, and any CPU, so a different worker count never changes the video.
- Native MP4 output supports the documented audio formats and encodes AAC. `--encoder ffmpeg` rejects compositions containing `<audio>`.
- Long exports can be split across processes or machines: `inspect <file> --fps N` prints the number of one-second segments, `render <file> --output part.aseg --segments A..B` renders one range with the same options on each worker, and `render <file> --output video.mp4 --parts a.aseg,b.aseg,...` (same `--variables`, `--audio-bitrate`) assembles them. Use it only when the user asks for distributed rendering; one process is simpler.
- `aether-motion inspect <composition.html>` can provide parsed composition details. Use it when timing or resolved structure is unclear.
- For reproducible output, state the selected size and FPS when they matter. A PNG snapshot is a single frame, not an MP4 export.
- Do not claim visual correctness from a successful check alone; inspect the rendered image or video.
