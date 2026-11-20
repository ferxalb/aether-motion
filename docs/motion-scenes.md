# Motion Scenes v1.2

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
- The preview watcher (`preview --watch`) follows every file the last successful compile read: the entry, each imported scene, and their images, image-sequence frames and folders, videos, audio, and caption transcripts, plus the variables file. After a failure it keeps watching the files of the last good compile and also watches every path the failed attempt read or tried to read, so creating a missing scene, transcript, video, sequence folder, or image triggers a new render. `check` lists the same files under `dependencies`. Fonts are built in and not listed.

## Reuse and limits

A scene is compiled once per distinct set of resolved `data-variable-values` (compared as JSON); further instances reuse the compiled scene instead of reading and parsing the file again. A scene without `data-hf-split-wrap` text compiles the same on both parses of a two-pass composition and is compiled once for both; a scene with it is reused only by a parse that gives its wrapping elements the same breaks. The result is identical to compiling every instance separately: element order, scoped IDs, `data-hf-split-wrap` breaks (renumbered and checked per instance), camera rigs, and audio are the same, and the depth, instance, and cycle checks still apply.

Instances share the compiled scene's element data. Each flattened element keeps its own scoped ID, tree links, and clip times; its content, style, motion tracks, and effects are shared with every other instance of the same compiled scene, and copied only where an instance differs: the root of a scene placed by `data-composition-fit`, and a camera that follows a subject (it points at its own instance's subject). Instances with different values, or of different files, share nothing. The renderer shares text layouts and glyph rasters between instances whose text and inherited styles come from the same shared elements, so a reused scene's text is rasterized once.

The flattened composition (the document plus every imported element, counted per instance) may hold at most 200,000 elements, checked as each instance is added. Split a larger film into separate renders.

`preview --watch` keeps the compiled scenes between renders. Before each compile, a kept scene is dropped if any file it read changed, appeared, or disappeared: its own document, the scenes it imports, and their assets (compared by size and modification time, and by content for files up to 1 MiB; folders by modification time and listing). Because a scene's files include those of its imports, an edit to a nested scene recompiles every scene that imports it, and unaffected scenes are reused. A scene that read a file modified within two seconds before the compile began is not kept, since the file may have changed while it was read. `check`, `render`, and the MCP tools always compile from scratch.

## Rendering cost

Inactive instances cost nothing to draw: an element outside its clip, or under an ancestor outside its clip, is never drawn. A frame also skips everything drawn before a scene instance that covers the whole frame with an opaque scene: its root has a solid background with full alpha and square corners, the host and root are fully opaque, unmoved enough that the root still contains the output frame, and drawn directly (no `filter`, `mix-blend-mode`, `clip-path`, `mask-image`, bloom, motion blur, rotation, partial reveal, or `overflow: hidden` and flex layout on the host). The skipped siblings would be completely overwritten, so frames are byte-identical; the skip applies only when drawing straight onto the frame, not inside offscreen layers, motion-blur samples, or the layout probe of `check`. Fades, crossfades, pushes, and letterboxed (`contain`) scenes keep everything drawn while they show what is below.

## Canvas size

When a scene's canvas differs from the containing composition, set `data-composition-fit` on the instance; without it the mismatch is an error.

- `contain` scales the scene uniformly to fit inside the canvas and centers it; the uncovered bands show the layers below (add a background clip for letterbox bars).
- `cover` scales it uniformly to fill the canvas and centers it; the overflow lies outside the frame. Add `overflow:hidden` to the instance if the instance itself moves.
- `none` centers it at its own size.

The scene root is laid out at its own size and placed with a static uniform scale around its center, so its layout, line breaks, `data-hf-split-wrap` measurement, camera rigs (subject tracking in the scene's own pixels), parallax, timing, and audio are unchanged; only the drawn picture scales. Text and vector artwork rasterize at the output size; raster images are resampled. `fill` (non-uniform stretching) is unsupported and returns an error. With matching canvases the attribute has no effect.

## Finishing

- Frame-wide finishing (root CSS color grade, exposure, highlight roll-off, shadow toe, vignette, and grain) and the root motion-blur shutter belong on the outer composition. For per-shot grading, put supported CSS color filters on an inner container. Imported scenes run entirely in Rust within the existing renderer.

[One Night](../examples/one-night/index.html) uses two independently authored scenes joined by a crossfade and one continuous sound bed.
