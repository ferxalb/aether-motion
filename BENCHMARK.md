# Initial measurements

Built with `cargo build --release`. Composition: [`examples/title-card.html`](examples/title-card.html), 4 s, 24 fps, 96 frames, rendered at 1920×1080 (`--scale 3`). Wall time and RSS were measured with system tools.

| Test | Internal time | Wall time | Peak RSS | Result |
| --- | ---: | ---: | ---: | --- |
| `bench` (drawing only) | 0.481 s | 0.55 s | 25,849,856 bytes | 28 frames drawn, 68 reused |
| Native MP4 `render`, 2 threads | 2.021 s | 2.12 s | 50,044,928 bytes | H.264 MP4, 366 KiB |

Final binary: 2,907,536 bytes. The MP4 was opened and transcoded with an external tool. `cargo test` and `cargo clippy --all-targets -- -D warnings` passed.

This measures a small composition dominated by static scenes. It does not measure performance on an older computer or CSS/GSAP effects that the native backend does not yet support. Do not extrapolate these results to other compositions or machines. To compare engines, use a representative composition and measure total time, RSS, visual quality, and output size on the same hardware.

## Native core comparison

Same machine and composition. Release binaries from the base commit and this revision. Median of five alternating runs per version at 1920×1080, 24 fps, and 96 frames. CLI internal time was used. MP4 was encoded with OpenH264 and two threads.

| Test | Before | Now | Time reduction |
| --- | ---: | ---: | ---: |
| `bench` (drawing only) | 0.465 s | 0.099 s | 79% |
| `render` (native MP4) | 1.876 s | 1.445 s | 23% |

All ten MP4 files had the same SHA-256. This comparison measures a composition with fades and static pauses. Results depend on the composition and the machine.

## Native MP4 at 60 fps

Both versions were measured on the same machine. The composition is three seconds long and contains text, solid blocks, translation, and opacity; it predates the example gradient. To measure 4K with both versions, a 960×540 input canvas and a 3840×2160 output were used. For 1080p, the input was 640×360. These are medians of three release runs per version, with 180 frames and `--threads 4`.

| Output | Before, drawing | Now, drawing | Before, MP4 | Now, MP4 |
| --- | ---: | ---: | ---: | ---: |
| 1920×1080 | 0.214 s | 0.094 s | 2.277 s | 1.908 s |
| 3840×2160 | 0.767 s | 0.171 s | 7.127 s | 6.131 s |

The tests verify MP4 frame cadence at 30 and 60 fps at both resolutions. The 4K export ran below 60 frames per second. The example with a gradient and rounded corners was also exported at 4K/60 fps and decoded after export.

## Parallel segment encoding

A 12 s composition with fades (`examples/title-card.html` with `data-duration="12"`), 60 fps, 720 frames, native MP4. Before: one GOP and `--threads 2`. Now: 1 s segments, each with its own encoder, and `--threads 4`. Three alternating runs per case.

| Output | Before | Now | RSS before | RSS now |
| --- | ---: | ---: | ---: | ---: |
| 1080p | 5.7–6.6 s | 2.2–2.5 s | 53 MB | 191 MB |
| 4K | 26.2–26.9 s | 9.6–9.7 s | 160 MB | 533 MB |

About a 2.7× improvement at both sizes. The tradeoff is memory: each thread holds its own frame and encoder. On machines with little RAM, use `--threads 1` or `2`.

- Segments are fixed at 1 s, so the MP4 is **byte-for-byte identical with any thread count** (verified with SHA-256 using 1, 2, 4, 8, and 12 threads, and by a test).
- Each segment starts with an IDR frame. The file is about 5% larger (843 KB vs. 800 KB at 1080p), and `--threads 1` takes about 10% longer than before.
- Compositions of 1 s or less are not segmented and keep the previous pipeline.
- By default, `--threads` uses the available cores, up to a maximum of 4.
- No comparison with other engines was made, and no measurement was taken on an older computer.

## Premium effects (camera, glass, grain, wipes)

Release build, drawing only (`bench`), 960×540 output (`--scale 0.5` of a 1920×1080 composition), 30 fps, single process. Median of five runs per row, using the CLI's internal time. Variants were made by deleting one effect from the same file, so the difference between two rows estimates that effect's cost. All frames change (continuous animation), so no frame is reused.

| Composition | Frames | Time | Per frame | Peak RSS |
| --- | ---: | ---: | ---: | ---: |
| [`premium-promo.html`](examples/premium-promo.html) (15 s, ~190 elements) | 450 | 14.71 s | 32.7 ms | 79,220,736 bytes |
| same, without `data-hf-vignette` and `data-hf-grain` | 450 | 10.23 s | 22.7 ms | 79,069,184 bytes |
| same, without `backdrop-filter` | 450 | 12.04 s | 26.8 ms | 75,251,712 bytes |
| [`glass.html`](examples/glass.html) (4 s) | 120 | 4.11 s | 34.2 ms | 21,221,376 bytes |
| same, without vignette and grain | 120 | 2.92 s | 24.3 ms | 21,434,368 bytes |
| same, without `backdrop-filter` | 120 | 2.83 s | 23.6 ms | 19,447,808 bytes |
| [`reveal.html`](examples/reveal.html) (4 s) | 120 | 0.21 s | 1.7 ms | 19,628,032 bytes |
| [`title-card.html`](examples/title-card.html) (4 s, baseline) | 120 | 0.02 s | 0.2 ms | 17,637,376 bytes |

What the differences suggest, on this single configuration:

- The frame-wide vignette and grain pass costs about 10 ms per 960×540 frame in both compositions. It touches every pixel every frame and disables the identical-frame shortcut, so it is the most expensive effect per pixel.
- The backdrop blur costs about 6 ms per frame in the promo (one to three glass cards visible) and about 10 ms in `glass.html` (a 40 px blur on a large card).
- These numbers measure continuously animated scenes with many elements, unlike the mostly static `title-card.html`; do not compare them with the earlier tables. No optimization has been made for these effects yet; profile before changing them.
- Peak memory stays under 80 MB at this size. Native MP4 export time was not measured here.

### After optimizing the vignette/grain pass and the backdrop blur

Same method, files, and settings as above (release, 960×540, 30 fps, median of five).

- The vignette is now a per-pixel brightness table built once per surface size and strength and cached on the surface; each frame applies it, and the grain, with integer arithmetic (no square root or float rounding per pixel).
- The blur's vertical pass now slides a running sum down the rows instead of gathering each column, so memory is read row by row.

| Composition | Before | After | Change |
| --- | ---: | ---: | ---: |
| `premium-promo.html` | 32.7 ms/frame | 22.3 ms/frame | −32% |
| `premium-promo.html`, without vignette and grain | 22.7 ms/frame | 20.8 ms/frame | −8% |
| `premium-promo.html`, without `backdrop-filter` | 26.8 ms/frame | 18.9 ms/frame | −30% |
| `glass.html` | 34.2 ms/frame | 22.5 ms/frame | −34% |
| `glass.html`, without vignette and grain | 24.3 ms/frame | 21.0 ms/frame | −14% |
| `glass.html`, without `backdrop-filter` | 23.6 ms/frame | 15.6 ms/frame | −34% |

Estimated cost of each effect now: vignette plus grain about 1.5 ms per frame (was about 10 ms); the 40 px blur in `glass.html` about 7 ms per frame (was about 10 ms). Peak memory is unchanged within a few MB (the vignette table adds 2 bytes per pixel). Rows that do not isolate an effect also moved by a few percent between runs, so treat differences under about 10% as noise. The vignette output can differ by one level per channel from the previous float version because of fixed-point rounding.

### Native MP4 export of the premium promo

`examples/premium-promo.html` (15 s, 450 frames, 30 fps), release build, native encoder, `--threads 2`, no audio.

| Output | Time | Peak RSS | File size |
| --- | ---: | ---: | ---: |
| 960×540 (`--scale 0.5`), median-free pair of runs | 6.1 s and 7.4 s | 138–140 MB | 841,201 bytes |
| 1920×1080 (`--size 1080p`), one run | 25.3 s | 265 MB | 2,068,017 bytes |

Both 540p exports have the same SHA-256, so output is deterministic. The 1080p file's container was parsed directly: 450 samples, 15.000 s at a timescale of 30, and a 1920×1080 track. No external player or transcoder was available, so decoding was not verified here. These are single runs; treat differences under about 10% as noise.

## Tile cache and lower default CPU use

Release build, drawing only (`bench`), `--scale 0.25` of a 1920x1080 composition (480x270), 30 fps, one run per row at `nice 10`; treat differences under about 10% as noise. "Before" is the build prior to this change.

| Composition | Frames | Before | After | Change |
| --- | ---: | ---: | ---: | ---: |
| [`premium-promo.html`](examples/premium-promo.html) | 450 | 2.69 s | 1.48 s | −45% |
| [`glass.html`](examples/glass.html) | 120 | 0.80 s | 0.40 s | −50% |

At 960x540 the first step (the radial glows) took `premium-promo.html` from 10.27 s to about 7.7 s before the cache was added. Earlier profiling showed the two full-frame radial glows were about half of the drawing time.

What changed:

- Square-cornered gradients (the drifting glows) and soft box shadows (sigma of 2 px or more) are rasterized once per size and blended on later frames. Each tile is fixed point (gradient color x256, alpha x4096; shadow coverage x65535), so output can differ from direct drawing by up to one level per channel, and a moving card's shadow snaps to whole pixels. Tests compare both paths.
- Backdrop blur, bit-identical to before: the horizontal pass has an interior loop without index clamping and blurs four rows in lockstep (independent running sums overlap instead of waiting on each other), the vertical passes ping-pong between two buffers instead of copying, the pixel gather and mix work row by row, and rounding no longer calls libm. Unit tests compare each piece with a reference, and frames of a composition that uses only `backdrop-filter` were byte-identical (PNG `cmp`) to the build before this change at frames 40, 150, and 300. Lockstep over 8 rows was no faster than 4.
- Remaining cost in `premium-promo.html` (attribution by removing one effect, 480x270): before the blur work `backdrop-filter` was about 0.47 s, glows about 0.33 s, and box shadows about 0.25 s of 1.48 s. After it, the whole composition takes about 1.3 s (1.29 and 1.33 s in two runs) and `backdrop-filter` about 0.3 s (1.30 s minus 1.02 s without it); `glass.html` at 960x540 went from 1.53–1.58 s to 1.24–1.35 s. The blur reads what changes every frame, so it cannot use the tile cache.

Native MP4 export of `premium-promo.html` at `--scale 0.25` (30 fps, 450 frames):

| Build | Workers | Wall time | User CPU time |
| --- | ---: | ---: | ---: |
| Before | 4 | 0.95 s | 3.22 s |
| Now (defaults) | 3 | 0.79 s | 2.03 s |

The default worker count is now a quarter of the cores (1 to 4) and the priority is lowered, so the MP4 is no longer byte-identical to earlier builds, though it is still identical for any `--threads` value. Measured once; a second measurement after the shadow cache and blur change was not repeated.

### Wide backdrop blur on a reduced copy

`backdrop-filter: blur()` with a sigma of 16 px or more now blurs a copy of the region reduced 2x or 4x (block averages), then scales it back up bilinearly. The reduction keeps the reduced sigma at 8 px or more, regions narrower than two sigmas stay at full size, and the three box widths are chosen so the blur has the same strength as the full-size path. This is the one step that is not bit-identical: it is checked against the full-size blur.

Quality, full-size blur versus reduced, on a stress scene (1920x1080 composition with a diagonal gradient, a white block, a black disc, and 160 px yellow text behind a 1100x500 glass card; the hard edges are the worst case for this technique):

| Blur sigma (composition px) | Output | Max error (of 255) | Mean error | Values differing | PSNR |
| --- | --- | ---: | ---: | ---: | ---: |
| 36 | 1920x1080 | 2 | 0.021 | 2.1% | 64.9 dB |
| 36 | 960x540 | 2 | 0.024 | 2.4% | 64.3 dB |
| 60 | 1920x1080 | 2 | 0.021 | 2.1% | 64.8 dB |
| 60 | 960x540 | 2 | 0.026 | 2.6% | 64.0 dB |
| 20 | 1920x1080 | 2 | 0.014 | 1.4% | 66.7 dB |
| 20 | 960x540 | 0 | 0 | 0% | identical (not reduced) |

Speed: the same scene animated (60 frames at 1920x1080, one run each, `nice 10`) took 5.2–5.4 s before and 1.94–1.99 s now; the same scene without the glass card takes 1.28 s, so the blur went from about 67 ms to about 11 ms per frame (about 6x). Unit tests bound the reduced blur to 3 levels on a 235-level step edge and verify the box widths reproduce the requested strength within 3%.
