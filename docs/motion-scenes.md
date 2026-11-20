# Motion Scenes v1

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

Give `coast.html` a 1920×1080 root and at least five seconds of authored duration. To use the example values, declare `headline` in that file's `data-composition-variables` and bind it with `data-var-text` or `var(--headline)`. Each instance resolves its own values against the source declarations. Omitted values use the source defaults. The containing composition can use `data-var-if` on an instance to decide whether to include it.

- The host can be an empty `div`, `section`, `article`, or `main`. Give it a unique `id`, a `data-start` (seconds or an ordinary local clip reference), and a `data-track-index`. `data-duration` defaults to the source duration. A shorter duration plays from local time zero and cuts picture and sound at the instance end. The instance must fit within its containing composition.
- The source is a local relative `.html` or `.htm` path. Dimensions must match the containing composition. Each document resolves relative paths from its own directory, including local assets supplied through variables. Nesting allows eight import levels and up to 256 instances per compilation. An import cycle through a symlink or relative alias fails.
- Each file has its own stylesheet, custom properties, typed variables, animation names, and reference scope. Timing expressions, camera subjects, transition sources, and audio ducking sources refer to IDs in the file that declares them. The native model and inspector qualify imported IDs as `opening::subject` or `opening::nested::subject`. Use the outer instance ID for a transition or timing reference in the containing file.
- The host and source root fill the containing canvas. Author the source root background if you need an opaque shot. Without one, the instance lets underlying layers show through. Put camera motion, parallax, and other animation on containers inside the source root. Apply transitions between instances using the native clip transition attributes. Put the incoming clip above the outgoing clip in draw order. Use `overflow:hidden` on an instance if overscanned artwork needs clipping as the instance moves.
- Instance fades and crossfades use the native renderer's inherited element opacity. For a full-picture fade to black over layered artwork, animate an opaque black overlay above the scene. Instance visual fades and transitions affect picture. Set audio fades on the audio clips that need them.
- The preview watcher tracks the entry file and variables file. Rerun preview after editing a scene or its assets.
- Frame-wide finishing (root CSS color grade, exposure, highlight roll-off, shadow toe, vignette, and grain) and the root motion-blur shutter belong on the outer composition. For per-shot grading, put supported CSS color filters on an inner container. Imported scenes run entirely in Rust within the existing renderer.

[One Night](../examples/one-night/index.html) uses three independently authored scenes and one continuous sound bed.
