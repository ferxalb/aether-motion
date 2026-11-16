# Porting from HyperFrames

AETHER Motion reads a documented subset of the HyperFrames composition format. Treat an existing HyperFrames project as source material to audit, not as a guarantee that every element will render in the native backend. See the [HyperFrames repository](https://github.com/heygen-com/hyperframes) for the upstream browser-based project.

## Keep the compatible parts

- Preserve root composition metadata and ordinary timed clips that use `class="clip"`, `data-start`, `data-duration`, and `data-track-index`.
- Keep styles and elements only when they are in AETHER Motion's current supported HTML/CSS contract. Existing supported `data-hf-*` attributes can usually remain; check their exact semantics in the target repository's `AGENTS.md`.
- Convert asset paths to local relative paths. Local PNG, JPEG, WebP, static SVG, and supported `<audio>` sources can be reused when their formats and markup meet the AETHER limits.
- Replace upstream timeline scripts or GSAP motion with native AETHER attributes or the limited CSS animation subset only when the same timing and result can be expressed. Preserve the source composition unchanged.

## Flag incompatible behavior

AETHER Motion does not run JavaScript, GSAP, `<iframe>`, canvas, or general browser CSS. `data-composition-src` sub-compositions are not supported by the native parser. `<video>` is supported for local H.264 MP4 files: map `data-media-start`, `muted`, and `data-volume` unchanged (the source's AAC track plays unless `muted`), and re-encode other codecs or open-GOP files. SVG works only as static artwork: text, animation, scripts, foreign objects, event handlers, and external references do not carry over. HyperFrames runtime adapters, shader transitions, DOM effects, and browser-only media behavior must be marked unsupported unless they can be reconstructed from the target contract.

Do not flatten unsupported moving footage into a still image and call it a conversion. If a material part of the source depends on unsupported behavior, explain the limitation and ask whether to produce a partial native version or keep that project in HyperFrames.
