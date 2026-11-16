---
name: aether-motion-import
description: Port an existing HyperFrames or Remotion composition into AETHER Motion when explicitly asked to migrate or convert it. Inspect the source, map only representable behavior, and document gaps.
---

# Import into AETHER Motion

Use this skill only when the user asks to port, migrate, or convert an existing HyperFrames or Remotion project into AETHER Motion. A new video inspired by an existing project belongs to the `aether-motion` creation workflow.

## Triage before translating

1. Read the target repository's `AGENTS.md` and `limits.md`. AETHER Motion implements a native subset of the HyperFrames format; it does not run a browser, React, JavaScript, or the upstream HyperFrames CLI.
2. Identify whether the source is a HyperFrames HTML project or a Remotion React project, then read the matching reference: [HyperFrames source](references/hyperframes.md) or [Remotion source](references/remotion.md).
3. Inventory dimensions, frame rate, duration, scene timing, assets, animation drivers, and media. Classify each important feature as directly representable, approximate, or unsupported before building the target.
4. Preserve the source files. Create a separate AETHER Motion composition and keep only the local assets that it can use.

Do not silently drop or materially change a scene, animation, or media track. If a central part depends on unsupported browser behavior, video, canvas, or dynamic React state, explain the gap and ask how the user wants to handle it. Record smaller approximations in `PORT_NOTES.md`; do not describe a lossy port as frame-identical.

## Build and verify the target

- Express the result with the supported root metadata, timed clips, HTML/CSS subset, native `data-hf-*` motion, static images or SVG, and supported local audio. Convert source frame times to seconds using the source FPS and preserve nested timing offsets.
- Choose an AETHER Motion output FPS and dimensions that the CLI supports. If the source uses fractional FPS or a setting that cannot be represented, surface that before changing the timing.
- Run `aether-motion check <file.html>` until `ok` is true. Inspect snapshots at each scene's key states and at transition boundaries. Render the final video only when the user requested a render, then inspect the result.
- If fidelity matters and the source project already renders, compare a few time-matched frames at the same size and FPS. A successful AETHER check proves contract validity, not equivalence to the source.
