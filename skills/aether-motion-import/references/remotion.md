# Porting from Remotion

Remotion renders React compositions from frame-indexed state. Read the actual source and its resolved composition metadata before translating; current projects may use `<Track>`, `<Series>`, `<TransitionSeries>`, or timing props on media components in addition to legacy direct `<Sequence>` usage.

Official references: [Composition](https://www.remotion.dev/docs/composition), [Track](https://www.remotion.dev/docs/track), [Sequence](https://www.remotion.dev/docs/sequence), [useCurrentFrame](https://www.remotion.dev/docs/use-current-frame), [interpolate](https://www.remotion.dev/docs/interpolate), and [TransitionSeries](https://www.remotion.dev/docs/transitions/transitionseries).

## Resolve timing first

- Read the selected composition's width, height, `fps`, and `durationInFrames`. Set AETHER root dimensions from width/height and calculate seconds as `frame / fps`.
- Remotion frames are zero-indexed. `useCurrentFrame()` inside a timed sequence returns a frame relative to that sequence; nested timing offsets accumulate. Convert the sequence's absolute start to `data-start` and convert animation points within the element's source clock to seconds relative to its AETHER clip.
- `durationInFrames` maps to `data-duration = durationInFrames / fps` for a single sequence at playback rate 1. Inspect `trimBefore`, `playbackRate`, `loop`, `freeze`, negative transition offsets, and inferred duration. Do not apply the simple mapping when these alter the source clock; resolve them explicitly or flag them as unsupported.
- AETHER Motion's CLI accepts integer FPS from 1 to 60. Do not silently round fractional FPS or move key events to different frames; explain the timing impact and get a choice when the source rate cannot be preserved.

## Map visual and motion primitives

| Remotion source | AETHER Motion approach |
| --- | --- |
| Static React/HTML content and fixed props | Rebuild as ordinary supported HTML, text, CSS, and local assets. Use separate elements for independently timed parts. |
| `<Track>` | Use AETHER `data-track-index` values for paint/timeline order; preserve overlapping clips on separate visual tracks as required. A track label has no direct equivalent. |
| `<Series>` or sequential timed clips | Resolve each child's start by accumulating preceding durations in frames, then divide by FPS. |
| `<Sequence from durationInFrames>` | Convert its start and duration from frames to seconds; account for parent and nested offsets before writing clip timing. |
| `useCurrentFrame()` + simple `interpolate()` | Turn source time/value stops into native keyframes only for supported outputs. `data-hf-keyframes` supports position offsets and opacity; limited CSS animation supports transforms and opacity. Check source extrapolation and easing instead of assuming they match. |
| `spring()` | AETHER's back easings are not the same physics model as a Remotion spring. Treat a hand-matched curve as an approximation and document it; do not claim exact equivalence. |
| `TransitionSeries` | Use an AETHER scene transition only when its type, direction, overlap, and easing can be represented by the target's transition contract. Model a simple overlay as a separate clip if its content is supported. |
| `<Img>` or static SVG shapes | Copy supported local images. Static SVG shapes can be used, but SVG text and internal animation cannot. |
| `<Audio>` | Use a supported local audio source with AETHER timing, trim, and fades. Variable volume automation is not supported. |
| `<Video>`, `<OffthreadVideo>`, canvas, Skia, Three, Lottie, or browser-only effects | These need runtimes the AETHER native backend does not provide. Mark them as blockers instead of replacing moving content with a still. |

Remotion's `interpolate()` can extrapolate beyond its input range unless the source requests clamping. AETHER keyframes hold their first and last values outside the keyed interval while the clip is active. Add explicit endpoints only when that preserves the source; otherwise record the mismatch.

## Preserve the source clock and report gaps

Prefer exact mapping of static geometry, timed scene boundaries, and simple opacity/position tracks. React state, effects, asynchronous metadata, network data, timers, or changing random values are not part of an AETHER composition; freeze them to explicit source values only when that reflects the user's intent.

If the original can already render and visual fidelity matters, sample matching frames from both projects at the same dimensions, integer FPS, and times. Record material differences in `PORT_NOTES.md`. A clean `aether-motion check` means the target is valid; it does not establish parity with Remotion.
