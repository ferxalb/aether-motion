# Motion Scenes v1.1

Write each scene as a standalone native HTML composition. Import it with an empty clip directly inside the root of another composition.

```html
<main data-composition-id="film" data-width="1920" data-height="1080" data-duration="9">
  <section class="clip" id="opening" data-composition-src="scenes/coast.html"
           data-start="0" data-duration="5" data-track-index="1"
           data-variable-values='{"headline":"Before dawn"}'></section>
  <section class="clip" id="ending" data-composition-src="scenes/coast.html"
           data-start="opening-1" data-duration="5" data-track-index="2"
           data-hf-transition="wipe-right" data-hf-transition-from="opening"
           data-variable-values='{"headline":"First light"}'></section>
</main>
```

Give `coast.html` a 1920×1080 root (or another size with `data-composition-fit`, below) and at least five seconds of authored duration. To use the example values, declare `headline` in that file's `data-composition-variables` and bind it with `data-var-text` or `var(--headline)`. Each instance resolves its own values against the source declarations. Omitted values use the source defaults. The containing composition can use `data-var-if` on an instance to decide whether to include it.

- The host can be an empty `div`, `section`, `article`, or `main`. Give it a unique `id`, a `data-start` (seconds or an ordinary local clip reference), and a `data-track-index`. `data-duration` defaults to the source duration. A shorter duration plays from local time zero and cuts picture and sound at the instance end. The instance must fit within its containing composition.
- The source is a local relative `.html` or `.htm` path. Each document resolves relative paths from its own directory, including local assets supplied through variables. Nesting allows eight import levels and up to 256 instances per compilation; reused instances count every time they appear. An import cycle through a symlink or relative alias fails, including a cycle reached only through a reused scene.
- Each file has its own stylesheet, custom properties, typed variables, animation names, and reference scope. Timing expressions, camera subjects, transition sources, and audio ducking sources refer to IDs in the file that declares them. The native model and inspector qualify imported IDs as `opening::subject` or `opening::nested::subject`. Use the outer instance ID for a transition or timing reference in the containing file.
- The host and source root fill the containing canvas. Author the source root background if you need an opaque shot. Without one, the instance lets underlying layers show through. Put camera motion, parallax, and other animation on containers inside the source root. Apply transitions between instances using the native clip transition attributes. Put the incoming clip above the outgoing clip in draw order. Use `overflow:hidden` on an instance if overscanned artwork needs clipping as the instance moves.
- Instance fades and crossfades use the native renderer's inherited element opacity. For a full-picture fade to black over layered artwork, animate an opaque black overlay above the scene. Instance visual fades and transitions affect picture. Set audio fades on the audio clips that need them.
- The preview watcher (`preview --watch`) follows every file the last successful compile read: the entry, each imported scene, and their images, image-sequence frames and folders, videos, audio, and caption transcripts, plus the variables file. A failed compile keeps watching the files of the last good one. `check` lists the same files under `dependencies`. Fonts are built in and not listed.
## Reuse and limits

A scene is compiled once per distinct set of resolved `data-variable-values` (compared as JSON) and parse pass; further instances clone the compiled scene instead of reading and parsing the file again. The result is identical to compiling every instance separately: element order, scoped IDs, `data-hf-split-wrap` breaks (renumbered and checked per instance), camera rigs, and audio are the same, and the depth, instance, and cycle checks still apply. Reuse is internal to one compile: nothing is cached on disk or between preview renders, so an edit is always picked up.

The flattened composition (the document plus every imported element, counted per instance) may hold at most 200,000 elements, checked as each instance is added. This bounds memory: reuse makes compiling cheap, but each instance still owns its elements for timing and layout. Split a larger film into separate renders.

## Canvas size

When a scene's canvas differs from the containing composition, set `data-composition-fit` on the instance; without it the mismatch is an error.

- `contain` scales the scene uniformly to fit inside the canvas and centers it; the uncovered bands show the layers below (add a background clip for letterbox bars).
- `cover` scales it uniformly to fill the canvas and centers it; the overflow lies outside the frame. Add `overflow:hidden` to the instance if the instance itself moves.
- `none` centers it at its own size.

The scene root is laid out at its own size and placed with a static uniform scale around its center, so its layout, line breaks, `data-hf-split-wrap` measurement, camera rigs (subject tracking in the scene's own pixels), parallax, timing, and audio are unchanged; only the drawn picture scales. Text and vector artwork rasterize at the output size; raster images are resampled. `fill` (non-uniform stretching) is unsupported and returns an error. With matching canvases the attribute has no effect.

## Finishing

- Frame-wide finishing (root CSS color grade, exposure, highlight roll-off, shadow toe, vignette, and grain) and the root motion-blur shutter belong on the outer composition. For per-shot grading, put supported CSS color filters on an inner container. Imported scenes run entirely in Rust within the existing renderer.

[One Night](../examples/one-night/index.html) uses three independently authored scenes and one continuous sound bed.
