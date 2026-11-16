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

## Per-frame profile at 1080p (one thread)

Native MP4 export of [`premium-promo.html`](examples/premium-promo.html) at `--size 1080p`, 30 fps, 450 frames, `--threads 1`, single run: 24.7 s wall time, 24.06 s user CPU, 267 MB peak RSS (about 53 ms of CPU per frame, or 1.6 CPU-seconds per second of video). One thread uses one core; the default `--threads` (a quarter of the cores) spreads the same CPU time over more cores.

Attribution per frame, from `bench --frame START --count 15` on the same file with one effect removed (one run each; differences under about 10% are noise). This was measured before the 2x/4x backdrop blur reduction was applied to these frames' build, so the blur row is an upper bound:

| Frames | Full | Without vignette and grain | Without glows | Without backdrop blur | Without box shadows |
| --- | ---: | ---: | ---: | ---: | ---: |
| 60–74 | 43 ms | 36 ms | 21 ms | 42 ms | 42 ms |
| 150–164 | 55 ms | 46 ms | 34 ms | 44 ms | 51 ms |
| 250–264 | 61 ms | 52 ms | 41 ms | 51 ms | 49 ms |

What this says:

- The two full-frame radial glows are the largest single cost (about 20 ms per frame, 35–50%), even with the tile cache. Alone, one cached glow (1400x1200 composition px) costs about 5.6 ms per frame at 1080p, so two cost about 11 ms; the rest of that difference is the full-frame area they mark as painted, which the next frame must clear.
- Vignette and grain cost about 7–9 ms per frame.
- The backdrop blur cost about 11 ms per frame while cards were visible.
- Two attempts that did not help and were reverted: keying the gradient tile by the element's own size, and a branch-free four-pixel blend loop. Neither changed the 1080p time, and the first broke the tile-versus-direct test by 2 levels. The glow cost therefore is not hash thrashing or branching in the blend loop.

Usage: `aether-motion bench <file.html> --size 1080p --frame 150 --count 15` draws 15 frames starting at frame 150 and prints the elapsed time, so a measurement costs about one second of CPU instead of a full render.

## Rejected exact grain lookup experiment

A 65,536-entry signed-noise lookup table was tested in place of the per-pixel
constant division. Three alternating release runs of `premium-promo.html`,
1080p, 30 fps, frames 150–169, one thread: baseline 0.958 / 0.942 / 0.955 s;
lookup 1.039 / 1.037 / 1.025 s. Median elapsed time increased by 8.6%.
Frame 150 PNGs were byte-identical. The experiment was reverted: preserving
quality alone is insufficient when the change makes rendering slower.
No measurements on older PCs were made.

## Exact lookup blending for rounded panel interiors

Rounded translucent fills now resolve constant-color RGB blending into three
256-byte tables, as square fills already do. Antialiased edges still use the
original coverage calculation; small fills retain the direct path. The tables
are local to a draw and require 768 bytes without a persistent cache.

Release, 1920x1080, 30 fps, one thread, frames 15–34 of
[`rounded-panel.html`](benchmarks/rounded-panel.html), three alternating runs:
baseline 0.190 / 0.208 / 0.196 s; updated 0.104 / 0.087 / 0.085 s.
Median reduction: 56% (0.196 to 0.087 s) for this large-panel workload.
Frame 20 PNGs are byte-identical. The targeted unit test compares all RGB
input levels across three opacities against the original interior formula,
including alpha output, and passes.

`glass.html`, 1080p, frames 40–59, three alternating runs after compilation:
baseline 0.806 / 0.787 / 0.844 s; updated 0.787 / 0.879 / 0.827 s.
The difference is within measurement noise; no general scene speedup is claimed.
An earlier batch overlapping test compilation measured baseline
0.853 / 1.229 / 0.937 s and updated 1.642 / 1.033 / 0.948 s, so it was
excluded from the comparison. Frame 40 PNGs are byte-identical.
No encoder settings, resolution, blur algorithms, or effects were changed.
Older PCs and total MP4 export time were not measured.

## Exact empty-span skipping for rounded borders

Border drawing now also skips the central rectangle guaranteed to lie fully
inside the rounded hole on curved rows. Potentially visible pixels retain the
original signed-distance and blending calculations. No cache or additional
pixel buffer is allocated.

Release build, one thread, 1920x1080, 30 fps, frames 15–34 of
[`rounded-border.html`](benchmarks/rounded-border.html), three alternating runs:
baseline 0.164 / 0.169 / 0.176 s; updated 0.097 / 0.111 / 0.101 s.
Median drawing time decreased 40% (0.169 to 0.101 s) in this large-radius
border workload. This is not a general MP4 speedup claim.

PNGs at frames 0, 20, 30, and 59 were byte-identical to the baseline. The
targeted `skipped_hole_matches_full_distance_scan` release test passed,
comparing full buffers with an exhaustive reference scan over four radii and
three widths, fractional geometry, and an offscreen edge. Composition checking
passed, and frame 20 was visually inspected. A 60-frame 1080p/30 fps native MP4
export with one worker completed in 1.013 s (one run, without a baseline MP4
timing; this export overlapped test compilation and is not a performance comparison).
No resolution, smoothing, effect strength, or encoder settings changed.
No older-PC measurements were made.

## Exact row-wise glyph painting

Glyph painting now clips once per glyph and zips destination rows with coverage
rows. This removes repeated coordinate checks and per-pixel destination offsets
from `blend_at`, preserving its floating-point operation order and alpha clamp.
No extra cache or buffer is allocated.

Release, one thread, 1080p, 30 fps, frames 15–34 of
[`animated-text.html`](benchmarks/animated-text.html), five alternating runs:
baseline 0.303 / 0.307 / 0.323 / 0.315 / 0.296 s; updated
0.290 / 0.291 / 0.283 / 0.288 / 0.285 s. Median drawing time decreased
6.2% (0.307 to 0.288 s). The stress scene deliberately overlaps large text
blocks and paints beyond frame edges; it is not a typography example.
An initial 96px-font variant measured baseline 0.143 / 0.133 / 0.157 s and
updated 0.129 / 0.135 / 0.144 s, too small a difference for a confident claim.

The targeted `clipped_glyph_rows_match_coordinate_blending` release test passed
against the original coordinate-based path, including all 256 coverage values,
nine opacity combinations, and clipping on all four sides. PNGs at frames
0, 30, and 59 and the entire 60-frame native MP4 were byte-identical. The
composition check passed and frame 30 was visually inspected. Single MP4
exports at 1080p/30 fps, one worker: baseline 2.749 s, updated 2.687 s; these
single runs do not establish an export speedup. No quality or encoder settings
changed, and older PCs were not measured.

## Exact zero-span skipping for inset shadows

Inset shadows now skip a conservative central strip on curved rows when it
is at least the Gaussian tail plus one pixel inside the rounded hole. The
coverage there is exactly one, making the shadow contribution exactly zero.
Existing straight-row behavior and all potentially visible pixel calculations
remain unchanged. No cache, downsampling, or extra pixel buffer is introduced.

Release, 1920x1080, 30 fps, one thread, frames 15–34 of
[`rounded-inset.html`](benchmarks/rounded-inset.html), three alternating runs:
baseline 0.216 / 0.213 / 0.215 s; updated 0.157 / 0.156 / 0.154 s.
Median drawing time decreased 27.4% (0.215 to 0.156 s). The baseline without
the inset shadow took 0.014 s in one attribution run, confirming this workload
is dominated by the shadow. These numbers apply to this large-radius scene.

All seven inset-shadow release tests passed. The new span test verifies
exactly zero shadow contribution for every skipped pixel across four origins
(including a fully offscreen hole), five radii, six sigmas including hard
edges, and fractional geometry. Composition checking passed, frame 30 was
visually inspected, and PNGs at frames 0, 30, and 59 were byte-identical.
The complete 60-frame native MP4 was also byte-identical. Single export
timings at 1080p/30 fps with one worker: baseline 1.409 s; updated 1.153 s.
These single export timings are not a repeatable MP4 speedup estimate.
No quality or encoder settings changed; older PCs were not measured.

## Direct filter-plane preparation and rejected shadow-mask experiment

`filter: blur()` now recovers premultiplied texels directly into the four blur
planes, sharing the same per-texel recovery helper with clipping. This removes
one intermediate RGBA allocation and its write/read pass (four bytes per layer
pixel). The Gaussian algorithm, sample values, and output composition are unchanged.

Release, one thread, 1920x1080, 30 fps, frames 15–34 of
[`animated-filter.html`](benchmarks/animated-filter.html), three alternating runs:
baseline 1.917 / 2.070 / 1.926 s; updated 1.847 / 1.871 / 1.903 s.
An initial baseline run was 1.896 s. Median drawing time moved from 1.926 to
1.871 s (2.9%), below the noise threshold; no CPU speedup is claimed.

A separate memory pair measured using system process statistics:

| Metric | Baseline | Updated |
| --- | ---: | ---: |
| Internal drawing time | 1.941 s | 1.962 s |
| Wall time | 2.00 s | 2.02 s |
| User CPU | 1.84 s | 1.85 s |
| System CPU | 0.15 s | 0.14 s |
| Maximum RSS | 151,134,208 bytes | 133,828,608 bytes |
| Peak memory footprint | 97,091,584 bytes | 88,395,776 bytes |

Maximum RSS fell by 17,305,600 bytes (11.5%) in this pair. Allocator behavior
affects the process-level saving; the guaranteed allocation removed is four
bytes per layer pixel. All five filter-blur and six overflow integration tests
passed. PNGs at frames 0, 30, and 59 were byte-identical, and composition checking
passed.

A separate experiment skipped the fully hidden central rectangle when building
outer-shadow masks. The exhaustive mask comparison passed, but the change was
reverted because the timing difference was too small. Release, one thread,
frames 15–34 of [`animated-shadow.html`](benchmarks/animated-shadow.html):
baseline 0.198 / 0.192 / 0.198 s; experiment 0.194 / 0.196 / 0.194 s.
An initial baseline was 0.198 s. Median difference: 2%, within measurement noise.

Inspection also confirmed that fitted images are already cached by source, size,
and object-fit, with a direct row-copy path for fully opaque images. Additional
image caching was not introduced. Gradient blending and grain remain candidates;
no new speedup for either is claimed here. No resolution or encoder settings
changed, and older PCs were not measured.

Final filter verification after reverting the shadow experiment: frame 30 was
visually inspected. Complete native MP4 exports (60 frames, 1080p, 30 fps, one
worker) were byte-identical: baseline 6.430 s; updated 5.823 s. These are single
export runs and do not establish a repeatable speedup.

## Exact full-opacity cached gradient blending at 30 and 60 fps

At full element opacity, cached gradient blending now uses integer arithmetic.
Cached colors are multiples of 1/256, alpha is a multiple of 1/4096, and Bayer
dither is a multiple of 1/32. Every term and sum of the original float blend
is therefore exactly representable; the integer path preserves rounding and
clamping. Animated element opacity below one retains the original float path.
Translucent gradient stops are supported by the integer path. No extra buffer,
CPU-specific instructions, reduced resolution, or encoder changes are used.

Release, 1080p, one thread, 20 frames starting at one second, three alternating
runs per row (seconds). All drawing measurements include the initial direct
draw and tile construction for the measured interval.

| Scene | FPS | Baseline runs | Updated runs | Median reduction |
| --- | ---: | --- | --- | ---: |
| [`animated-glows.html`](benchmarks/animated-glows.html) | 30 | 0.779 / 0.763 / 0.766 | 0.510 / 0.500 / 0.514 | 33.4% |
| Same | 60 | 0.776 / 0.768 / 0.758 | 0.512 / 0.501 / 0.502 | 34.6% |
| `premium-promo.html` | 30 | 0.758 / 0.747 / 0.746 | 0.655 / 0.652 / 0.646 | 12.7% |
| Same | 60 | 0.706 / 0.705 / 0.704 | 0.605 / 0.596 / 0.596 | 15.5% |

The 30 fps interval is frames 30–49; the 60 fps interval is frames 60–79.
They start at the same time but cover different durations. The glow fixture
moves by integer output pixels at both frame rates to isolate blending from
cache-key changes. A preliminary variant moving by half a pixel per frame
at 60 fps measured baseline 0.791 s at 30 fps and 1.361 s at 60 fps;
it was replaced before the optimization comparison because changing rasterized
widths exercises tile-cache budget behavior as well as blending.

The exact integer-versus-float blend test passed across all destination byte
values, all 16 Bayer phases, representative color and alpha extremes, and
transparent texels with nonopaque destination alpha. The existing cached-versus-
direct test also passed (its historical tolerance remains unchanged).
`exports_full_hd_and_uhd_with_exact_cadence` passed: 30/60 fps at 1080p/4K,
checking sample count, per-sample timestamp and duration, track size, and total
duration. These cadence tests do not benchmark 4K glow performance.

The glow composition check passed and its 60 fps midpoint was visually inspected.
Complete native MP4 exports of the two-second glow scene, one worker, were
byte-identical to baseline at each frame rate (60 frames at 30 fps; 120 at 60 fps).
Single export timings: 30 fps baseline 2.412 s, updated 1.575 s; 60 fps baseline
4.357 s, updated 2.688 s. These single export pairs do not establish repeatable
end-to-end speedups. Drawing results are scoped to the measured intervals;
older PCs and sustained realtime performance were not measured.

## Five-area exact-output optimization audit (30 and 60 fps)

All candidate changes were measured at 1080p, one thread, low priority, 20
frames starting at one second (frame 30 at 30 fps; frame 60 at 60 fps).
Three alternating baseline/updated drawing pairs were run for every row.
The intervals start together but cover different durations at the two rates.
[Raw measurements](benchmarks/element-optimization-results.json) preserve the
initial experiments, final confirmation runs, memory pair, and MP4 checks.

Retained changes:

- Gradient stop colors are premultiplied once per draw rather than per pixel.
  This reduces work when fractional motion changes tile sizes and the existing
  cache falls back to direct painting. Cache keys, admission, and budget remain
  unchanged: admitting formerly uncached tiles could change pixels because the
  historical cached and direct paths are not generally byte-identical. This
  optimization does not remove the underlying cache-budget limitation.
- Cached glows at element opacity 0.25, 0.5, and 0.75 use exact integer blending.
  Every intermediate of the corresponding float formula is representable
  exactly, including dither and rounding. Arbitrary-opacity fades retain their
  original float path; no general fade speedup is claimed.
- Fitted images cache the nontransparent span and opaque status of each row.
  Fully transparent margins are skipped; opaque spans at opacity one use a
  direct copy; partial alpha keeps its original blend. Metadata counts toward
  the existing 64 MiB image-cache budget and is optional, preserving the
  maximum previously legal fitted-image size.

Final confirmation after removing rejected experiments:

| Scene | FPS | Baseline runs (s) | Updated runs (s) | Median reduction |
| --- | ---: | --- | --- | ---: |
| [fractional-glows.html](benchmarks/fractional-glows.html) | 30 | 0.522 / 0.508 / 0.517 | 0.476 / 0.459 / 0.463 | 10.4% |
| [fractional-glows.html](benchmarks/fractional-glows.html) | 60 | 1.194 / 1.189 / 1.195 | 1.045 / 1.047 / 1.035 | 12.5% |
| [half-opacity-glows.html](benchmarks/half-opacity-glows.html) | 30 | 0.774 / 0.765 / 0.765 | 0.473 / 0.470 / 0.476 | 38.2% |
| [half-opacity-glows.html](benchmarks/half-opacity-glows.html) | 60 | 0.772 / 0.772 / 0.782 | 0.468 / 0.470 / 0.469 | 39.2% |
| [translucent-image.html](benchmarks/translucent-image.html) | 30 | 0.200 / 0.196 / 0.197 | 0.101 / 0.099 / 0.097 | 49.7% |
| [translucent-image.html](benchmarks/translucent-image.html) | 60 | 0.199 / 0.197 / 0.201 | 0.096 / 0.098 / 0.097 | 51.3% |

An initial fractional-glow batch showed a smaller 6.2% difference at 30 fps
(0.513 vs 0.481 s medians); the final batch showed 10.4%. Treat that particular
30 fps improvement as modest and sensitive to measurement variation.

Rejected and reverted:

- Separate compile-time loops for grain, vignette, and their combination: all
  six scene/rate pairs moved by less than about 5%, within measurement noise.
  The fixed-point reference comparison passed before this experiment was reverted.
- A 32 MiB offscreen-pixel pool for blur, rotation, and clipping: both nested-
  layer scene/rate pairs moved by less than 1%. A separate 60 fps memory pair
  measured baseline RSS 33,415,168 bytes and experiment RSS 33,505,280 bytes;
  there was no demonstrated CPU or memory benefit. Pool reset and capacity
  tests passed, but the pool and its integration were removed.

Validation:

- The experimental painting-domain run passed 40 tests, including the rejected
  finish/pool tests. The filter-blur, mixed-alpha image, native rendering, and
  overflow integration suites passed 34 tests. Native tests include exact
  30/60 fps cadence at 1080p and 4K, frame ordering, rotation, and parallel
  segment determinism. This does not benchmark 4K performance.
- After reverting rejected code, all three gradient tests and the mixed-alpha
  image regression test passed again. The stop test compares float bit patterns;
  the quarter-opacity test compares the original float blend with the integer
  blend across all Bayer phases and representative source/destination values.
  The image test includes transparent rows, opaque rows, partial alpha, opacity
  changes, and offscreen clipping.
- All eight benchmark compositions passed structure checking. Complete two-
  second native MP4s for fractional glows, half-opacity glows, continuous fading
  glows, mixed-alpha images, grain, vignette, combined finishing, and nested
  layers were byte-identical to baseline at both 30 and 60 fps (16 comparison
  pairs). Direct container inspection verified 60 or 120 samples, a one-tick
  duration per sample, timescale matching fps, and two-second track duration.
- The half-opacity glow and mixed-alpha image midpoints were visually inspected
  at 1080p/60 fps. Final compilation and whitespace checks passed.

Single final MP4 export pairs (1080p, one thread); these are correctness exports,
not repeated end-to-end performance estimates:

| Scene | FPS | Baseline (s) | Updated (s) | Bytes (identical) |
| --- | ---: | ---: | ---: | ---: |
| fractional-glows | 30 | 1.555 | 1.482 | 25816 |
| fractional-glows | 60 | 6.074 | 5.426 | 28238 |
| half-opacity-glows | 30 | 2.321 | 1.468 | 23574 |
| half-opacity-glows | 60 | 4.311 | 2.602 | 26259 |
| fading-glows | 30 | 2.587 | 2.500 | 106310 |
| fading-glows | 60 | 4.552 | 4.508 | 123161 |
| translucent-image | 30 | 1.099 | 0.805 | 35659 |
| translucent-image | 60 | 2.103 | 1.504 | 56031 |

These figures apply to synthetic workloads, not all compositions. No resolution,
quality, noise, blur, dithering, or encoder setting was reduced. Rendering speed
is separate from the exported video's frame rate; sustained realtime rendering
and performance on older PCs were not established.

## Cache metadata and checked-allocation stability

This change adds resource checks without changing rendering quality or encoder
settings. [Validation records](benchmarks/stability-validation-results.json)
preserve the before/after files' hashes, sample counts, output sizes, single
export times, targeted test results, and drawing regression measurements.

- Gradient and outer-shadow caches cap all keys at 4,096 each, including Seen
  and Direct entries. Exhaustion produces an explicit error, without evicting
  existing tiles or silently changing the cached/direct drawing decisions.
  An existing Seen entry can still be promoted at the cap. Oversized tile costs
  cannot wrap the budget arithmetic or invoke the allocation builder.
- Fitted-image caches use their existing FIFO eviction at 4,096 entries; text
  runs prepare uncached results after 4,096 entries. This bounds metadata even
  for tiny images or runs with no glyph bytes. Text-layout block keys were
  already capped; their quantization slack now uses saturating addition.
- `render/allocation.rs` centralizes checked RGBA multiplication and coordinate
  conversion. The existing 128 MiB per-frame/per-layer byte limit is preserved.
  Rotated/filter layer extents subtract in i64 before conversion to usize.
  Filter reach and margins use checked arithmetic; extreme and nonfinite
  layer coordinates return errors before allocating.
- Gaussian shadow-edge profiles reject invalid radii or profiles over 16 MiB
  before building the table. Shadow compositing uses widened offset arithmetic.
  Strongly negative spread cannot cast a negative mask dimension to usize;
  zero-sized masks remain eligible for their original empty cached tile.

The initial rendering-domain run passed 58 unit tests; selected filter, image,
inset-shadow, native, overflow, and extreme-size integration suites passed
38 tests. Tests cover key exhaustion, preserved existing slots, oversized cost
arithmetic, image/text entry caps, extreme layer geometry, invalid shadow
profiles, and negative spread. Native tests include exact 30/60 fps cadence
at 1080p and 4K and deterministic parallel segments.

An export-time variation prompted a focused drawing check on half-opacity
glows: 20 frames at 1080p, one thread, starting at one second, three alternating
pairs at each fps. Before enforcing inlining of the exact integer quarter-
opacity blend, baseline/new runs were 0.481 / 0.478 / 0.472 versus
0.559 / 0.546 / 0.539 s at 30 fps, and 0.488 / 0.484 / 0.486 versus
0.554 / 0.555 / 0.556 s at 60 fps. After the inline annotation, baseline/new
runs were 0.481 / 0.484 / 0.473 versus 0.495 / 0.486 / 0.491 s at 30 fps,
and 0.473 / 0.476 / 0.479 versus 0.489 / 0.494 / 0.493 s at 60 fps. The
remaining median differences (2.1% and 3.6%) are within the measurement noise
threshold; no performance improvement is claimed. All three gradient tests
passed again, and both half-opacity MP4 comparisons remained byte-identical.

Six two-second scenes (fractional glows, half-opacity glows, outer shadows,
inset shadows, nested layers, and mixed-alpha images) passed composition
checking. Their complete native MP4s were byte-identical to the pre-change
binary at both 30 and 60 fps: 12 comparison pairs, at 1080p with one worker.
Container inspection verified 60/120 samples, one tick per sample, matching
fps timescale, and two-second duration. Single export timings are retained
in the raw records for completeness and do not establish a speed difference.

These bounds apply to individual resources and cache entry counts, not total
process memory. Allocation failure under system-wide memory pressure and
performance on older PCs were not validated.

A final boundary review retained zero-sized shadow masks as valid empty tiles
while rejecting only negative mask dimensions from the tile builder. All three
shadow tests passed afterward, including a new empty-mask regression test;
both 30/60 fps outer-shadow MP4 comparisons passed again. The nested-layer
midpoint was visually inspected at 1080p/60 fps. The final set comprises 97
distinct selected tests across the initial run and the additional boundary test.

## Temporal stability correction (2026-11-15)

The monochrome promo exposed spatial quantization during its slow camera zooms:
text was re-rasterized at every zoom value, glyph positions were rounded individually,
and fractional box bounds expanded to integer edges. Soft-shadow tiles also rounded
geometry that the first, uncached draw retained. Fixed-resolution glyph/shadow masks
now use bilinear coverage sampling for translation and zoom. Fractional solid fills
and general gradients retain floating-point geometry; fractional outer shadows stay
on the direct path so cache admission cannot move them.

This is a visual correctness change, not a speed optimization. A release, one-worker
sample of `examples/aether-monochrome-promo.html`, frames 120–134 at 60 fps,
1920×1080, took 0.952 s before and 1.498 s after. A corrected-build run during
concurrent export/test compilation took 4.137 s and is not comparable to the idle
sample. These are single observations, not a controlled performance study. The new
coverage sampling and direct fractional paint paths cost additional CPU work.

The complete corrected 30-second 1080p/60 fps export with two workers took
115.693 s under concurrent validation load and produced 12,369,948 bytes. The
original export was 9,847,903 bytes. Both files contained 1,800 samples at timescale
60 with duration one tick and zero rendering offset; all 1,800 samples decoded.
The original container therefore already had uniform cadence: the reproduced
stability defects were in rasterization, rather than MP4 sample scheduling.

All 55 selected integration tests passed (motion stability, native rendering/export,
gradients, text shadows, spacing, and wrapping). The five temporal regressions cover
quarter-pixel motion at 60 fps, constant box area, stable text coverage/centroids,
continuous camera text zoom, cache-independent fractional shadows, and a decoded
120-frame MP4 crossing a segment boundary. The promo passed composition checking;
its 2.5-second snapshot was visually inspected at 1080p.

## Audio

Measured with the release build and `--threads 1` on a 64×36 static composition, so the time is almost all audio. The source is 60 s of stereo 16-bit 44.1 kHz WAV, mixed to 48 kHz and encoded to AAC-LC at 192 kbit/s. Without `<audio>` the same render takes 0.06 s.

| Stage | Time for 60 s of audio |
| --- | ---: |
| Decode, resample 44.1 to 48 kHz, and mix (`check`, 10 s of audio: 0.12 s) | about 0.7 s |
| AAC-LC encoding with upstream `oxideav-aac` 0.1.7 (direct `cos` per MDCT term) | about 96 s |
| AAC-LC encoding with the vendored table-driven MDCT | about 19 s (3 runs: 20.3, 19.8, 19.8 s) |

Peak memory for the 60 s case is about 96 MB (the mix and the decoded source are held as `f32`), against 11 MB without audio. The 753,080-byte MP4 is identical in size before and after the MDCT change. AAC encoding costs about one third of real time on one thread and runs before video encoding, so it adds to the render time; a 15-second composition with a full-length track adds about 5 s. Decoder-side verification: the burst-position tests in `tests/audio.rs` and a manual decode of the MP4 with the operating system's AAC decoder both place a 10 ms burst at the expected sample (centroid 35999.99 against 36000).
