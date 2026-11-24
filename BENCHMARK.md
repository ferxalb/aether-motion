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

- Segments are fixed at 1 s, so the MP4 is **byte-for-byte identical with any thread count** (verified with SHA-256 using 1, 2, 4, 8, and 12 threads, and by a test). Correction: this holds for compositions without cached gradient tiles. `examples/premium-promo.html` gives different hashes for `--threads 1` and `--threads 4` (also before the distributed-rendering change), with differences of one level per channel; see "Encoding" in `docs/architecture.md`. Fixed in 0.1.5: every thread count and split is byte-identical (see the 0.1.5 section).
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

- Gradient and outer-shadow caches cap all keys at 65,536 each, including Seen
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

## Video clips

Measured with the release build and `--threads 1` (`bench --frame START --count N`) on a 1920×1080, 30 fps, 15 s H.264 source with 15 keyframes (a GOP of 30 frames, no B-frames) made by this renderer's encoder, inside a 1920×1080 composition.

| Case | Time per frame |
| --- | ---: |
| Source and box both 1920×1080 (`fill`, no resize): decode, YUV to RGBA, blit | about 7 ms (60 frames: 0.41 to 0.45 s) |
| 1920×1080 source into a 1280×720 `object-fit: cover` box, `imageops::resize` (Triangle) | about 43 ms (60 frames: 2.61 s) |
| The same with the integer separable triangle resize in `paint/resample.rs` | about 23 ms (60 frames: 1.37 s, two runs identical) |
| 15 frames starting 15 frames into a GOP (decode from the keyframe first) | about 9 ms (0.13 s) |

The full 15 s render of the downscaled case takes 15.1 s with `--threads 1` and 5.7 s with `--threads 3`, with byte-identical MP4s. The resize replacement differs from `imageops::resize` by at most 6 levels per channel on test gradients (a unit test checks this); the resize dominates the cost whenever source and box sizes differ, so matching them (`fill` at the source size) is the cheap path. A request that goes backward or jumps ahead restarts decoding at the keyframe of its GOP, so cost grows with GOP length; sequential rendering decodes every frame once.

## FFT MDCT, call-free pixel rounding, and quality presets (2026-11-16)

Release builds, one thread, `nice 10`, on a shared 4-core cloud container (wall times there vary by
about ±10% between runs, so instruction counts from `valgrind --tool=callgrind` are reported too).
"Before" is the build at the start of this change.

### AAC: FFT-based forward MDCT

The vendored `oxideav-aac` evaluated its forward MDCT as an O(N²) sum over a cosine table (about
two million multiply-adds per channel and frame). Every AAC-LC transform length (2048 and 256) is a
power of two, so it now folds the input into a DCT-IV and evaluates that with one N/4-point complex
FFT. Other lengths keep the table sum. Against the direct formula the coefficients agree to about
1e-13 relative (the test bound is 1e-11, for lengths 8 to 2048 and the 960 fallback).

| Case (64×36 video, so the time is almost all audio) | Before | After |
| --- | ---: | ---: |
| 60 s stereo WAV at 44.1 kHz, looped, to AAC-LC 192 kbit/s, native MP4 | 16.7 s | 1.43 s (two runs: 1.43, 1.41) |

The MP4s are byte-identical, as are the MP4s of `examples/audio.html` and `examples/video.html` (whose
clip carries an AAC track). AAC now costs about 2.4% of real time per track instead of 28%.

### Drawing: exact call-free `round` and `hypot`

On the baseline x86-64 target, `f32::round` is a call into the soft-float runtime and `f32::hypot`
calls libm. A callgrind profile of `premium-promo.html` (frames 280–282, 1080p) put about 16% of
instructions in `roundf` and 5% in `hypotf`, nearly all from the direct (uncached) gradient loop, which
fractional glow motion uses every frame. `paint/float_ops.rs` now holds `round_to_u8` (moved from the
backdrop blur, exact for every input, also used by `blend_pixel` and small translucent fills) and an
inlined `hypot` that evaluates `sqrt(x² + y²)` in `f64`, the same evaluation as glibc's `hypotf`
(100 million random inputs and a unit test: no bit differs). The gradient loop also hoists its
per-row dither and vertical coverage.

| Composition (frames 280–282, 1080p) | Instructions before | After | Change |
| --- | ---: | ---: | ---: |
| `premium-promo.html` | 3.450 G | 3.056 G | −11.4% |
| `aether-monochrome-promo.html` | 4.500 G | 4.006 G | −11.0% |

`bench --frame 60 --count 20` at 1080p, three runs each (seconds): `premium-promo` 1.555 / 1.475 /
1.462 before, 1.393 / 1.462 / 1.264 after (median −5.6%); `aether-monochrome-promo` 1.935 / 1.923 /
2.067 before, 2.036 / 1.799 / 1.711 after (−7.0%); `glass` 1.462 / 1.371 / 1.530 before, 1.295 /
1.328 / 1.339 after (−9.2%). These are within the noise threshold individually; the instruction
counts are the firmer evidence. PNGs of every example and benchmark composition at frames 0, 17, 45,
90, and 200 (136 frames at `--scale 0.5`) are byte-identical to the previous build.

Rejected: replacing every `round`/`floor`/`ceil` under `src/render` with a generic exact inline
version (truncate, then correct the fraction and the sign of zero). It executed as many instructions
as the runtime calls it replaced, and `glass.html` drew 20–30% slower with both a branching and a
branch-free correction; it was reverted. Only the specialized, already clamped `round_to_u8` pays off.

### Quality presets

With rate control off, OpenH264 encodes every frame at the layer QP `iDLayerQp`, which the `openh264`
crate never set: exports always used QP 26, and the configured `24..32` range only bounds
adaptive-quantization changes to P frames. Changing that range produced byte-identical files. The
crate initializes the encoder inside its first `encode_at`, so the wrapper is vendored with a
`base_qp` option (`vendor/openh264/AETHER-PATCH.md`), and `--quality` picks QP 30, 26, 22, or 18.

Fidelity against the lossless PNG frames (FFmpeg `psnr` on luma and `ssim`, references converted to
yuv420p by FFmpeg), 1080p, 30 fps, one thread; size in bytes:

| Composition | QP 26 (`standard`) | QP 22 (`high`) | QP 18 (`max`) |
| --- | --- | --- | --- |
| `kinetic-text.html` (5 s) | 249,904 B, 51.98 dB, SSIM 0.99892 | 329,731 B, 55.42 dB, 0.99936 | 425,624 B, 58.08 dB, 0.99961 |
| `camera.html` (6 s) | 495,553 B, 52.95 dB, 0.99864 | 666,900 B, 54.82 dB, 0.99893 | 843,552 B, 54.62 dB, 0.99901 |
| `motion.html` (3 s) | 272,640 B, 52.22 dB, 0.99886 | 336,574 B, 53.68 dB, 0.99928 | 406,152 B, 54.22 dB, 0.99953 |
| `transitions.html` (9 s) | 453,862 B, 49.17 dB, 0.99699 | 844,928 B, 51.22 dB, 0.99739 | 831,849 B, 50.24 dB, 0.99721 |
| `glass.html` (4 s, film grain) | 346,785 B, 43.37 dB, 0.97774 | 507,374 B, 43.90 dB, 0.97827 | 1,998,077 B, 43.77 dB, 0.97735 |

SSIM rises with every step except on the two outliers: `transitions.html`, where OpenH264's
mode decisions in fast slides make QP 22 slightly better than QP 18 to 21, and `glass.html`, whose
per-frame grain is incompressible noise, so the error floor stays near 43.5 dB while the file grows.
Turning off background or scene-change detection did not change that pattern. `Complexity::High`
(on `transitions.html` at QP 18 to 24) made files 1% to 11% smaller with luma PSNR within 0.07 dB,
at 1% to 12% more encoding time, so the encoder stays at `Complexity::Low`.

Full native export of `premium-promo.html` (450 frames, 1080p, 30 fps, `--threads 1`, one run each):
before 41.6 s and 2,347,124 bytes; `--quality standard` 38.5 s and the same bytes (byte-identical
file); `--quality max` 39.4 s and 6,617,373 bytes. The quantizer barely changes CPU
time because drawing dominates (37.7 s of the export is drawing). Both files decode to 450 frames.
The default stays `standard`: the higher presets mostly grow the file (SSIM gains of 0.0003 to 0.0007
above, none on grain or fast slides), which was not judged worth the size.
No measurements were made on older PCs or other operating systems.

## Filters, blend modes, root grade, and motion blur (2026-11-16)

New features, so there is no earlier timing to compare against; the numbers below set the baseline and
show that existing compositions are untouched. All runs: `bench --threads 1`, 1080p, 30 fps, release
build, three runs each (seconds).

Existing output: PNGs of every example and benchmark composition at frames 0, 20, and 50 (111 frames
at `--scale 0.5`, plus 66 at full size) and the `glass.html` MP4 are byte-identical to the previous
build. `glass.html` `--frame 60 --count 20`: 1.511 / 1.586 / 1.540 before, 1.524 / 1.559 / 1.556
after (within noise; the only change on that path is one extra check per element).

`examples/effects.html` (`--frame 10 --count 20`) with motion blur removed costs 2.623–2.727 s, against
0.369–0.387 s for the same scene with every filter and blend mode removed. With the first version of
the effects code, removing single features showed where the time goes: the three blended elements
(about 1.1 megapixels of layers, each drawn on black and on white) about 78 ms per frame, the
two-function root grade about 45 ms per frame.

A callgrind profile of frames 10–12 (one thread) guided three exact optimizations; instruction totals:

| Step | Instructions | Change |
| --- | ---: | ---: |
| First version | 4.442 G | |
| Blend loop monomorphized per mode (no indirect call per channel), one division per pixel, `round_to_u8` instead of `round`, prefix of channel-wise grade steps read from per-channel tables, last grade result reused for repeated pixels | 4.051 G | −8.8% |
| Layer planes filled through iterator zips into zeroed allocations instead of indexed writes | 3.673 G | −17.3% in total |

The grade tables hold the exact float results of the channel-wise steps for each 8-bit input, and a
unit test compares the table path with the direct float path on every input value, so the output is
unchanged by these optimizations. Still dominant: drawing gradients twice per layer (black and white
passes recover premultiplied alpha), which the existing filter blur shares.

Motion blur multiplies drawing by the sample count on frames where something moves: the same 20 frames
take 16.3–17.0 s with `data-motion-blur-samples="8"` and 34.1–34.5 s with automatic samples (the puck
travels fast enough to ask for more), against 2.6–2.7 s sharp. Static frames are drawn once.

## Animated filters (2026-11-16)

Same settings as the section above (`bench --threads 1`, 1080p, 30 fps, release build).

Existing output: the 111 half-scale PNGs of every example and benchmark composition (frames 0, 20,
and 50) and three complete MP4s (`aether-intro-landscape`, `glass`, `premium-promo`, half scale,
sequential frames with warm caches, including static blur layers) are byte-identical to the build
before the effects work. `examples/effects.html` frames 0, 10, 18, 30, 60, and 89 are byte-identical to
the first effects build, so the optimizations below did not change the plane path either. Instruction
counts for `premium-promo.html` frames 280–282: 3.056 G before, 3.051 G after (the larger
`FrameState`, with eight filter amounts per element, costs nothing measurable); wall-clock medians of
five alternating runs differed by up to 6% in either direction, which is noise on this machine.

`examples/filter-motion.html` (`--frame 18 --count 20`: a full-frame scene animating `saturate` and
`brightness`, a CSS `hue-rotate` loop on a radial orb, and a blurred title focusing in):

| Step | Seconds (three runs) |
| --- | --- |
| First animated-filter build | 4.893 / 4.783 / 4.757 |
| Color-only layers recovered, recolored, and composited in one pass instead of through four `f32` planes (33 MiB per full-frame layer) | 4.339 / 4.434 / 4.438 |
| Effect layers share the frame's gradient and shadow tile caches, frozen during both passes | 2.526 / 2.568 / 2.629 |

A callgrind profile of the second step put about 45% of instructions in gradient rasterization inside
the layers: every layer was a fresh `Surface` with empty tile caches, so a full-frame gradient was
sampled twice per frame (black and white passes). Sharing the caches needs both passes to agree,
because the cached tile and the direct path may differ by one level; `TileCache::freeze` keeps the
decisions identical (a unit test replays two passes), and an integration test renders six frames with
one renderer and with a fresh renderer per frame and finds at most one level of difference, the same
tolerance as the frame's own cache. Static-blur-only layers keep fresh caches so existing output does
not change. A test also checks that the single-pass color path and the plane path give identical pixels.

`examples/effects.html` without motion blur (`--frame 10 --count 20`) went from 2.6–2.7 s to
1.83–2.06 s with the same two changes.

## Composition variables (2026-11-16)

Variables are resolved and their bindings applied once, on the parsed document, before the stylesheet
is read; nothing in the per-frame path changed. With no `data-composition-variables` the pass is one walk
over the document that finds no binding attributes, and the root gets no extra custom properties.

Existing output: the same 111 half-scale PNGs (every example and benchmark composition, frames 0, 20,
and 50, `--scale 0.5`) rendered with this build are byte-identical to the reference set from the build
before the effects work. Parse cost: `inspect examples/premium-promo.html`, mean of 20 runs, 5 ms with
the reference build and 4 ms with this one (process start included; the difference is noise).
`examples/variables.html` renders one 960x540 frame in 0.05 to 0.07 s with its defaults, with
`examples/variables.json`, and with an inline `--variables` object.

## Audio effects (2026-11-17)

Volume automation, equalizer, and ducking run on the calling thread during the mix, before encoding;
nothing in the per-frame path changed, and clips without effects take the unchanged sample copy (a test
checks that a neutral `data-hf-volume-ramp` gives a byte-identical mix).

Method: `check --safe-zone off` (decode, resample, mix, no encoding) of a 600 s composition with the
`examples/audio` voice (16 kHz mono) and music (22.05 kHz stereo) looped over the whole timeline, so each
effect processes 28.8 million samples per channel. Three runs each, one thread, release build:

| Variant | Before optimization | After |
| --- | --- | --- |
| No effects | 1.12–1.18 s | 1.15–1.25 s |
| `data-hf-eq` on both clips (4 + 3 bands) | 2.17 s | 1.67–1.82 s |
| `data-hf-duck` on the music | 1.99 s | 1.80–1.88 s |
| `data-hf-volume-ramp`, one `ease-out-cubic` segment over 600 s | 14.07 s | 1.98–2.00 s |
| All three (the `examples/audio-effects.html` setup) | 3.09–3.36 s | 2.57–2.71 s |

The first version evaluated the automation curve per sample, and a `cubic-bezier` easing costs a numeric
solve per call (about 450 ns). The curve is now read at control rate: every 32 samples (1.5 kHz) with
linear interpolation inside the block, which is exact on held and linear segments; a segment whose two
gains are equal skips the easing. Ducking gains use one `exp` per ramp sample instead of `powf`, and the
equalizer flushes tiny filter states once per block instead of branching per sample. With every effect
on, the mix costs about 0.4% of real time on one core.

## Distributed rendering (part files)

[`premium-promo.html`](examples/premium-promo.html) (15 s, every frame changes), 960×540 (`--scale 0.5`), 30 fps, 450 frames, `--threads 4`, release build, one 4-core container, wall time of single runs. The four ranges were rendered one after another to time each as if it ran on its own 4-core worker.

| Run | Wall time |
| --- | ---: |
| Single render, `--threads 4` (two runs) | 3.46 s, 3.43 s |
| Part `0..4` / `4..8` / `8..12` / `12..15` | 0.75 s / 1.47 s / 1.30 s / 0.89 s |
| Assembly of the four parts (`--parts`) | 0.007 s |

With four workers the export finishes when the slowest part does: about 1.48 s instead of 3.45 s (2.3×). The ranges are equal in seconds, not in work, so the busiest scenes bound the gain. This excludes worker start-up and file transfer, which the API amortizes by sharding only long renders. The assembled file differed from the single render by 12 bytes (gradient tiles, see the correction above); `examples/title-card.html` stretched to 9 s assembled byte-identically.

## 0.1.5: exact tiles, branch-free kernels, and progress

Release build, drawing only (`bench --size 1080p --frame START --count 20`), one thread, `nice 10`,
three alternating runs of the 0.1.0 baseline and 0.1.5 on the same 4-core x86-64 virtual machine with
AVX2 (seconds; the median is in bold). Measured after the final code change of this release.

| Scene (start frame) | 0.1.0 runs | 0.1.5 runs | Median change |
| --- | --- | --- | ---: |
| [`premium-promo.html`](examples/premium-promo.html) (150) | **1.848** / 1.891 / 1.836 | 1.534 / **1.602** / 1.627 | −13% |
| [`glass.html`](examples/glass.html) (40) | 1.462 / **1.367** / 1.360 | 1.155 / 1.293 / **1.156** | −15% |
| [`grain.html`](benchmarks/grain.html) (30) | 0.218 / 0.198 / **0.203** | 0.093 / 0.086 / **0.101**¹ | −54% |
| [`vignette.html`](benchmarks/vignette.html) (30) | **0.111** / 0.128 / 0.101 | **0.091** / 0.089 / 0.090 | −19% |
| [`fractional-glows.html`](benchmarks/fractional-glows.html) (30) | 0.636 / 0.549 / **0.605** | 0.478 / 0.503 / **0.482** | −20% |
| [`animated-glows.html`](benchmarks/animated-glows.html) (30) | **0.543** / 0.591 / 0.538 | **0.494** / 0.523 / 0.472 | −9% |
| [`animated-shadow.html`](benchmarks/animated-shadow.html) (15) | 0.233 / **0.214** / 0.201 | 0.268 / **0.222** / 0.215 | +4% (noise) |
| [`animated-text.html`](benchmarks/animated-text.html) (15) | 0.841 / 0.931 / **0.875** | 0.942 / **0.875** / 0.863 | 0% |
| [`flex-cards.html`](examples/flex-cards.html) (30) | 0.252 / **0.259** / 0.300 | **0.256** / 0.254 / 0.294 | −1% (noise) |

¹ Median 0.093 s.

What changed, and how exactness was checked:

- **Determinism.** An uncached draw of a whole-pixel gradient or soft shadow now computes, for its
  visible pixels, the same fixed-point texels a cached tile holds and blends them with the same code,
  so cache decisions no longer change pixels. `premium-promo.html` at `--scale 0.5` gave different
  MP4 hashes for `--threads 1` and `--threads 4` with 0.1.0 and the same hash with 0.1.5 (also with
  `AETHER_SIMD=off`). `tests/distributed.rs` covers 1, 2, and 4 threads and a two-part split. The
  cost: first draws of those layers can differ from 0.1.0 by one level per channel (two in a few pixels
  inside filtered layers); across 93 frames of every example at `--scale 0.5`, the largest difference
  was 2 levels and the largest mean absolute difference 0.57/65535.
- **Kernels.** `render/paint/kernels.rs` holds branch-free row loops (two-stop gradient blending,
  radial gradient parameters, shadow masks) compiled for the baseline target and for AVX2 and chosen
  at run time; `finish.rs` dispatches the vignette and grain rows the same way. No FMA and no
  reassociation, so both builds give the same bits: unit tests compare each kernel with the loop it
  replaced, and the 93 example frames were byte-identical between the AVX2 path, the scalar path
  (`AETHER_SIMD=off`), and the build before the kernels. Directly drawn radial gradients also skip,
  unevaluated, the pixels proven to sample a transparent last stop (with a one-pixel and 1e-4 margin).
  Instruction counts (callgrind, 4 frames of `premium-promo.html` at 1080p) fell from 3.70 G to 3.06 G.
- **Rejected.** Moving the cached-tile blend at full opacity into a dispatched kernel (it then wrote
  the alpha of every pixel) lowered instruction counts but not wall time, and was reverted. Evaluating
  tile texels pixel by pixel through `Mapper::t` made the first frame of `animated-glows.html` 80%
  slower (0.19 s against 0.10 s); evaluating whole rows through `Mapper::row` brought it to 0.071 s.
- **Shadows.** Uncached whole-pixel shadows skip the element's hidden interior and texels too faint to
  pass the 0.002 blend threshold, as the direct path did. `animated-shadow.html` and `flex-cards.html`
  still execute 2.4% and 2.0% more instructions than 0.1.0 (callgrind), within timing noise.

`<video>` decoding (not measured before): a 4 s, 1920x1080, 30 fps High-profile H.264 source without
B-frames, drawn full frame at 1080p, took 0.706 / 0.740 / 0.691 s for 30 frames starting at frame 15
(about 23 ms per frame for decoding, color conversion, and the opaque blit; the first frames also decode
from the GOP's keyframe). The same composition with a still PNG took 0.024 to 0.028 s, because its
frames are reused. Sources with B-frames fail in the bundled OpenH264 decoder with an out-of-memory
state and are now rejected when the composition is parsed.

Progress reporting is throttled to one update per 250 ms and touches only an atomic counter per
frame; it does not change the output (`--progress json` and `off` gave the same MP4 hash).

## 0.1.6: faster finish pass, kinetic text, bloom, and per-element motion blur (2026-11-18)

Machine: a 4-core x86-64 virtual machine with AVX2 and 15 GiB of memory, no GPU. Release builds
(`lto = "thin"`, one codegen unit) of the starting revision (0.1.5, `a5dc90b`) and of this change,
run one after the other, never in parallel with a build, at `nice 10`. Wall and CPU time and peak
RSS come from `getrusage` of the child process.

### Where 4K60 export time goes (shared scene)

`AETHER_PROFILE=1` on [`aether-monochrome-promo.html`](examples/aether-monochrome-promo.html), segments
5..8 (180 frames, 3 s) at `--size 4k --fps 60 --threads 1`, before the optimization:

| Phase | Thread time | Per frame | Share |
| --- | ---: | ---: | ---: |
| Drawing (`raster`) | 57.84 s | 321.4 ms | 89.2% |
| RGBA to YUV (`convert`) | 1.12 s | 6.2 ms | 1.7% |
| H.264 (`encode`) | 5.91 s | 32.9 ms | 9.1% |

Inside drawing, callgrind (3 frames from frame 360 at 4K, 11.98 G instructions) put the vignette and
grain finish pass first among single passes: 1.49 G instructions (12.5%, about 60 per pixel),
followed by rotated layers (13.4% including their content), text-shadow masks (10.7%), and two-stop
gradient rows of the drifting glows (11.7%). Repeated text layout was not a cost: text blocks and runs
are already cached per element, and layout lookups did not reach 1%. Temporary buffers were not
either: the largest per-frame allocation is the frame-sized sum of the shutter, unused here.

### The optimization: the finish pass

The row loops of `render/paint/finish.rs` were meant to run in an AVX2 copy, but they were passed to
`for_each` as closures, which the compiler emits as separate functions without the target feature.
The rows now use plain loops, hash eight pixels side by side, shade each pixel as a `u32` lane, and
divide the noise by 65535 with an exact 32-bit form; unit tests compare the rows with the per-pixel
loop and the division with `/`. Glyph and shadow masks also compute each destination column's source
position once per paint (same arithmetic).

| Measure (callgrind, 3 frames at 4K) | 0.1.5 | 0.1.6 |
| --- | ---: | ---: |
| Finish rows | 1.493 G | 0.202 G |
| Text-shadow painting | 1.286 G | 0.925 G |
| Whole run | 11.977 G | 10.216 G |

Drawing only (`bench --size 4k --fps 60 --frame START --count 20`, one thread), three alternating runs
(seconds; median in bold):

| Start frame | 0.1.5 runs | 0.1.6 runs | Median change |
| --- | --- | --- | ---: |
| 120 | **5.347** / 5.336 / 5.391 | 4.826 / 4.638 / **4.764** | −10.9% |
| 900 | **4.463** / 4.400 / 4.512 | 4.000 / **4.015** / 4.039 | −10.0% |
| 1500 | 5.035 / 5.203 / **5.023**¹ | 4.541 / **4.523** / 4.455 | −10.2% |

¹ Median 5.035 s.

Full export of the shared scene (`render --size 4k --fps 60 --threads 2`, 1800 frames, 30 s):

| Build | Wall | CPU | Peak RSS | MP4 size |
| --- | ---: | ---: | ---: | ---: |
| 0.1.5 | 273.6 s | 531.1 s | 562 MiB | 33,107,987 B |
| 0.1.6 | 246.4 s | 477.0 s | 552 MiB | 33,107,987 B |

The two MP4s are byte-identical (`cmp`), as are 4K PNG stills at frames 30, 400, 700, 1000, 1300, and
1700, also with `AETHER_SIMD=off`.

### The 0.1.6 demo and the cost of the new effects

[`aether-motion-016-demo.html`](examples/aether-motion-016-demo.html) (10 s, wrapped kinetic letters,
four bloom elements, one element with its own motion blur), `render --size 4k --fps 60 --quality max
--threads 2`: 600 frames in 78.0 s wall, 152.9 s CPU, 759 MiB peak RSS, 5,062,576 bytes. The file
plays at a constant 60 fps (600 samples 1/60 s apart, 10.000 s); export throughput on this machine was
7.7 frames per second. Thread time from `AETHER_PROFILE=1` on the same export:

| Phase | Thread time | Calls | Per call |
| --- | ---: | ---: | ---: |
| Drawing (`raster`) | 135.1 s | 600 frames | 225.2 ms |
| of which effect layers | 64.4 s | 2278 | 28.3 ms |
| of which bloom halos | 41.7 s | 2278 | 18.3 ms |
| of which per-element motion blur | 10.1 s | 251 blurred draws | 40.2 ms |
| RGBA to YUV | 4.0 s | 600 | 6.7 ms |
| H.264 | 15.1 s | 600 | 25.1 ms |

Bloom is the most expensive new effect here: its halos are large (a 150 px title with a 70 px radius)
and each glowing element is drawn twice into its effect layer. Wide halos already run on a 4x
reduced grid; the full-resolution bright pass and the upsampling add stay proportional to the layer.

`--quality standard` (QP 26) gave 2,848,324 bytes but visibly stepped the darkest halo falloffs: the
encoder removes the renderer's ±1 ordered dither in near-black gradients. QP 18 (`max`) reduces the
blocks; along a 360 px line through the ring's halo the decoded frame still descends in 31 steps of at
most 2 luma levels, where the rendered frame dithers between levels 203 times. Adding
`data-hf-grain="0.02"` did not help (the encoder flattens it too) and cost 3% more time, so the demo
does not use grain.

## Cinematic still and plate reuse

Measured on macOS x86-64 on a desktop CPU, using the modified release binary. Frame 72
at 24 fps from [The Morning Shift](examples/cinematic-dystopia.html) was written as a native
3840×2160 PNG in 5.41 s wall time, including encoding and I/O. The process used 5.14 s user CPU
plus 0.21 s system CPU (99% of one logical core on average) and peaked at 257,564,672 bytes
of RSS. A direct snapshot and a snapshot after frame 70 produced identical PNG bytes.

At 1920×1080, 15 drawing-only frames starting at frame 64 took 22.301 s with the original
oversized near rain plate and 2.652 s once the near field used four placements of one
cached tile. This measures the two versions of this example on the same engine. The engine's
64 MiB fitted-image budget was held fixed. A one-off legacy workload check on
`examples/premium-promo.html` used the same frame range and measured 0.797 s on the
pre-existing 0.1.5 executable and 0.649 s on this build. The executable versions differ by
unrelated changes, so the latter comparison does not isolate the cinematic changes.

## Fitted SVG cache pressure at 4K

Native macOS x86-64 on a desktop CPU. The previous release binary was preserved
and measured before changing the cache. Both release builds used the same scene and a
single render thread. The 1080p case is drawing-only at 24 fps over 15 frames starting at
64. The 4K batch renders and writes frames 72, 84, and 96 at 24 fps from
`examples/cinematic-dystopia.html`. CPU is user plus system time and RSS is the process
maximum reported by `/usr/bin/time -lp`.

| Workload | Wall before → after | CPU before → after | Peak RSS before → after |
| --- | ---: | ---: | ---: |
| 1080p, 15 frames | 2.65 → 2.66 s | 2.62 → 2.62 s | 103,350,272 → 102,887,424 bytes |
| 4K, three PNGs | 15.30 → 6.21 s | 15.15 → 6.10 s | 293,466,112 → 202,305,536 bytes |
| 4K, isolated frame 72 | 5.19 → 5.31 s | 5.16 → 5.26 s | 269,172,736 → 178,405,376 bytes |

The 4K batch reduced wall time by 59.4%, CPU time by 59.7%, and peak RSS by 31.1%.
The 1080p cache had zero evictions and stayed raw. An isolated snapshot still pays for the
first SVG rasterization and packing, which traded 0.12 s wall time for 33.7% lower peak RSS
in this run.

A controlled run of the modified binary compared `AETHER_PROFILE=1 AETHER_IMAGE_PACK=off`
with packing enabled. All three PNGs from both modes matched the original continuity
references byte-for-byte.

| Three 4K frames, same binary | Raw cache | Packed on pressure |
| --- | ---: | ---: |
| Fitted-image hits / misses / evictions | 9 / 54 / 51 | 44 / 19 / 0 |
| SVG raster calls / total time | 51 / 13.752 s | 17 / 4.447 s |
| SVG parse time | 0.164 s | 0.059 s |
| Image fitting time | 0.019 s | 0.013 s |
| Image blit time, including unpacking | 0.538 s | 0.702 s |
| Image packing time | — | 0.086 s |
| Finishing time | 0.305 s | 0.315 s |
| Largest requested fitted raster | 52,828,440 bytes | 52,828,440 bytes |
| Total requested fitted raster bytes | 739,404,348 | 247,430,504 |
| Peak worker fitted-cache bytes | 67,015,332 | 59,650,148 |

The raw cache spent 91% of its measured raster phase on SVG rasterization. The optimized
cache retained those exact pixels within the existing 64 MiB budget, shifting a small
amount of work into blitting. A small, cache-pressure fixture produced byte-identical
segmented MP4s at 1, 2, 4 and 64 configured threads and with packing disabled.

## Cinematic short production validation

Native macOS x86-64 on a desktop CPU. The existing release build rendered the original
`examples/cinematic-dystopia.html` before production changes. CPU totals combine user
and system time. The original 4K reference frames 72, 84, and 96 matched byte-for-byte.

| Phase 1 run | Wall | CPU | Peak RSS | Fitted-image hits / misses / evictions | SVG rasters |
| --- | ---: | ---: | ---: | ---: | ---: |
| 4K draw, frames 64–78, 1 thread | 10.82 s | 10.58 s | 201,129,984 bytes | 290 / 25 / 0 | 17 |
| 4K animated zoom with parallax, three PNGs 70–72, 1 thread | 16.07 s | 15.89 s | 237,826,048 bytes | 10 / 53 / 0 | 50 |
| Original 1080p MP4, 192 frames, 2 threads, high | 15.62 s | 30.71 s | 263,069,696 bytes | 3,974 / 58 / 0 | 34 |

The zoom probe changed the camera from 1.026× to 1.10× over eight seconds. Its SVG
rasters took 13.850 s because the exact fitted dimensions changed, even though the cache
did not evict entries. The original export's sampled live RSS plateaued around
227–239 MiB after startup. AVFoundation decoded all 192 frames at 1920×1080, exactly
24 fps with correct presentation timestamps and an eight-second duration.

The production scene uses three fixed shot zooms with moving camera focus. Near geometry,
the walker, light masks, and reflection share one parallax factor so they stay registered.
Tiled rain keeps the near and far rain placements within the fitted-image budget. The 1080p
preview decoded all 360 frames with correct cadence. It took 35.57 s wall, 68.20 s CPU, and
296,435,712 bytes peak RSS at two threads and high quality.

The final native 4K master uses the max quality preset and three threads. It rendered
360 frames in 103.04 s wall (3.49 frames/s), with 296.96 s user CPU, 4.60 s system CPU,
and 1,042,837,504 bytes peak RSS. The file is 562,600,610 bytes. The profile recorded
213.426 s of raster work, including 57.608 s across 171 SVG raster calls, and 85.867 s
of encoding. The fitted cache served 10,497 hits, with 423 misses and 123 evictions over
all workers. The 171 SVG raster calls are consistent with warming the artwork at three
shot sizes on three workers. Cache usage stayed within the 64 MiB fitted-image budget
per worker. The measured process RSS includes the render surfaces and native encoders.

The native frame 300 PNG took 6.77 s wall and 6.74 s CPU with 216,211,456 bytes peak RSS.
AVFoundation decoded all 360 master frames at 3840×2160 with correct 24 fps presentation
timestamps and an exact 15-second duration.

### Into the Shadows 1080p revision

On the same native macOS setup, the revised scene rendered all 360 frames in 25.75 s
wall (13.98 frames/s), using 48.17 s user CPU, 0.41 s system CPU, and 281,980,928
bytes peak RSS. The previous 1080p preview took 35.57 s wall and 68.20 s CPU at the
same two-thread, high-quality settings. The revision uses independently drawn shots.
The measurements compare the two compositions on the same engine.

The revised export had 8,466 fitted-image hits and zero evictions, with 54 SVG
rasterizations across both workers. AVFoundation decoded all 360 frames at 1920×1080
with the expected 24 FPS timestamps and an exact 15-second duration.

### Into the Shadows 4K master validation

Same native macOS setup and release binary that wrote `out/cinematic-walk-4k-v3.mp4`
(`render examples/cinematic-walk-15s.html --size 4k --fps 24 --quality max --threads 4`).
The master itself was not re-exported.

| Check | Result |
| --- | --- |
| Container | H.264 (`avc1`), 3840×2160, 24 fps, timescale 24, 15.000 s, 395,392,612 bytes |
| Samples | 360 coded, 15 sync (one per second), decode order equals presentation order |
| Timestamps | 0 to 14.958 s, every step exactly 1/24 s, no duplicates |
| AVFoundation decode | 360 of 360 frames, reader completed without error (26.5 s wall) |
| Reproducibility | 1-second 4K segments 0–1, 5–6, and 10–11 rendered with the command above: 96 of 96 sampled 4 KiB slices found verbatim in the master |
| Master vs. lossless render | PSNR 38.5–42.8 dB on frames 0, 119, 121, 125, 129, 241, 242, 300, 359 |
| Thread determinism | 15 4K PNGs (both transitions, frames 0, 300, 359) identical at 1 and 4 threads; cold single-frame renders identical to batch |
| Segmented render | half-scale parts split at 5 s and 10 s assembled byte-identical to a single render |

Per-frame luma statistics over a 160×90 grid show the largest frame-to-frame changes
only inside the two concealed shot changes (mean absolute change 13.6 of 255 at frame 123
under the column, 13.3 at frame 238 as the fog rises); elsewhere it stays at or below 2.4
until the final fade, which falls smoothly to black by frame 353. Frames 113–132 and
233–252 were inspected as stills; playback in real time was not reviewed.

Drawing-only 4K benchmark, one thread, `AETHER_PROFILE=1`, 20 frames each:

| Window | Wall | CPU | Peak RSS | Layer blur | SVG rasters | Fitted hits / misses / evictions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Frames 112–131 (wipe and column) | 32.86 s | 32.37 s | 729,153,536 bytes | 13.477 s | 22 (5.491 s) | 853 / 67 / 0 |
| Frames 232–251 (fog crossfade) | 14.77 s | 14.59 s | 243,351,552 bytes | 0 s | 10 (2.633 s) | 450 / 52 / 0 |

The column's per-element motion blur (12 samples) accounts for 41% of the first window's
raster time. Peak RSS does not grow with frame count: frames 122–123 alone peaked at
837,656,576 bytes and frames 112–151 at 615,669,760 bytes, so the peak is the transient
working set of the blurred column at 4K. Fitted-image peaks stayed under the 64 MiB budget
(56,655,136 and 56,950,556 bytes) with zero evictions.

Known limitation at the time of this measurement: the native encoder converted with
limited-range BT.601 and tagged the stream accordingly (SMPTE C primaries), which was
consistent but not the BT.709 convention usual for HD and UHD. Resolved by the BT.709
conversion recorded under "Native BT.709 output" below.

## Camera Rig v1 (2026-11-19)

`examples/cinematic-camera-rig-6s.html` (1920×1080, 24 fps, 144 frames, `--quality standard`,
release build with `AETHER_PROFILE=1`, same desktop machine as the cinematic sections above).
The baseline is the same composition with the rig replaced by classic `data-hf-camera`
keyframes at the same four key times, zooms, and resolved focus points (`ease-in-out-cubic`
segments). It runs on the same binary, so drawing, caching, and encoding are identical. Only
the camera path differs.

| Run | Wall | CPU (user + sys) | Peak RSS | Effective fps | SVG rasters | Fitted hits / misses / evictions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Rig, 1 thread | 229.22 s | 229.17 s | 275,173,376 bytes | 0.63 | 1,683 | 2,336 / 1,984 / 1,680 |
| Classic baseline, 1 thread | 212.38 s | 212.34 s | 264,781,824 bytes | 0.68 | 1,581 | 2,463 / 1,857 / 1,555 |
| Rig, 4 threads | 75.97 s | 262.46 s | 763,166,720 bytes | 1.90 | 1,757 | 2,219 / 2,101 / 1,193 |
| Classic baseline, 4 threads | 76.84 s | 254.74 s | 770,052,096 bytes | 1.87 | 1,687 | 2,311 / 2,009 / 1,124 |

Rig evaluation does not appear in the profile. A pose is a few curve evaluations and one
spline read of the baked path, and the bake (240 Hz over 6 s) runs once at load. The 1-thread
difference follows the SVG raster count. The rig's log-space dolly yields different fitted
plate sizes than the baseline's eased zoom, and that costs 102 more SVG rasters (+6.5%); at 4
threads the wall times are within 1%. Every SVG raster
happens on a zooming frame: frames 4–23 (zoom held at 1.35 while the camera tracks) did 19
cold rasters, 0 evictions, and 551 hits on both cameras. Pans and holds therefore reuse the
cache. Zooming frames fit each plate at a new exact size, as classic zoom always has. This
cost is pre-existing and is not addressed in this phase (no cache redesign). The cache stays
within its 64 MiB budget per worker (peak 67,108,804 bytes), so memory is bounded.

Determinism, on the demo itself:

| Check | Result |
| --- | --- |
| 1 vs 4 threads | MP4s identical (`32992c86…`); the baseline is also identical across thread counts |
| Segmented export | 1-second segment parts `4..6`, `0..2`, `2..4`, rendered in that order by separate processes and assembled, are byte-identical to the single render |
| Frame order | 1080p PNGs of frames 0, 47, 48, 95, 96, and 143 are identical when rendered as one batch (4 threads), in reverse order (1 thread), and one process per frame |
| Camera curve (`tests/camera_rig.rs`) | Sampled every 1 ms: velocity changes at most 1.5 px/s per ms in translation and 0.01/s per ms in scale. Zoom is held for 0–2 s, rises for 2–4 s, and falls for 4–6 s, monotonically in each segment. The shot ends at rest on its key; the follow-to-fixed blend reverses by at most 0.0014 px in total |
| Framing | While following (0–2 s), the walker's center stays 189.4–195.6 px left of the frame center (anchor 192 px) and exactly on the vertical anchor. The ±3.1 px ripple is the walk's own 3.75 px steps at zoom 1.62; the camera itself moves smoothly |

Native 4K PNGs (`--size 4k`, 1 thread):

| Frame | Wall | CPU | Peak RSS |
| --- | ---: | ---: | ---: |
| 0 (follow) | 8.70 s | 8.68 s | 249,483,264 bytes |
| 48 (dolly starts) | 8.76 s | 8.74 s | 250,097,664 bytes |
| 72 (dolly) | 9.99 s | 9.97 s | 285,614,080 bytes |
| 96 (deepest zoom) | 11.50 s | 11.48 s | 328,704,000 bytes |
| 120 (reveal) | 8.06 s | 8.05 s | 224,067,584 bytes |
| 143 (end) | 5.74 s | 5.72 s | 158,412,800 bytes |

The first 4K draft kept the dawn sky inside the camera (`data-hf-parallax="0"`). At zoom 1.6
its fitted raster exceeded the existing 64 MiB per-image limit, and frame 96 failed with
`rasterized image exceeds 64 MiB`. The sky is now a fixed backdrop outside the camera, which is
also physically right for a dolly. The largest fitted raster at 1080p dropped from 22,965,660
to 13,850,172 bytes.

## Zoom-aware SVG reuse (2026-11-19)

Release builds of the commit before the zoom ladder (baseline) and of the ladder, measured in the
same session on a 4-core Linux container (so absolute times differ from the desktop sections
above), `AETHER_PROFILE=1`, `--quality standard`, 24 fps. CPU is user plus system time and RSS is
the process maximum (`getrusage`). Every pair uses identical commands and thread counts.

`examples/cinematic-camera-rig-6s.html`, full 1080p MP4 export (144 frames):

| Run | Wall | CPU | Peak RSS | Effective fps | SVG rasters | Fitted hits / misses / evictions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| Baseline, 1 thread | 184.53 s | 182.69 s | 216,760,320 bytes | 0.78 | 1,683 (150.8 s) | 2,336 / 1,984 / 1,680 |
| Ladder, 1 thread | 57.00 s | 56.42 s | 211,279,872 bytes | 2.53 | 128 (11.5 s) | 3,897 / 2,541 / 961 |
| Baseline, 4 threads | 58.95 s | 200.80 s | 696,029,184 bytes | 2.44 | 1,757 (167.8 s) | 2,219 / 2,101 / 1,193 |
| Ladder, 4 threads | 22.89 s | 66.43 s | 730,923,008 bytes | 6.29 | 238 (24.7 s) | 3,738 / 2,309 / 1,693 |

At one thread, SVG rasterizations fall 92% and wall time 69%. The new cost is resampling: 2,118
calls, 22.5 s (10.6 ms each at 1080p). Most misses and evictions are now resampled fits, which are
cheap to remake and are evicted before any exact raster. At four threads, peak RSS rises 5%
(each worker holds its own ladder levels inside its own 64 MiB budget). A rerun of the final
binary took 53.38 s at one thread and 21.09 s at four.

Drawing-only windows (`bench`, 1 thread):

| Window | Wall | CPU | Peak RSS | SVG rasters | Resample | Fitted hits / misses / evictions |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| 1080p frames 48–67 (dolly), baseline | 34.53 s | 34.25 s | 139,231,232 bytes | 347 (29.9 s) | — | 199 / 401 / 196 |
| 1080p frames 48–67 (dolly), ladder | 10.06 s | 9.98 s | 147,144,704 bytes | 37 (3.7 s) | 3.6 s | 509 / 451 / 349 |
| 1080p frames 4–23 (zoom held, camera tracks), baseline | 4.22 s | 4.19 s | 108,814,336 bytes | 19 | — | 551 / 49 / 0 |
| 1080p frames 4–23 (zoom held, camera tracks), ladder | 4.32 s | 4.30 s | 106,704,896 bytes | 19 | — | 551 / 49 / 0 |
| 4K frames 88–95 (deepest dolly), baseline | 67.79 s | 67.29 s | 267,235,328 bytes | 145 (61.1 s) | — | 73 / 167 / 96 |
| 4K frames 88–95 (deepest dolly), ladder | 23.59 s | 23.38 s | 256,937,984 bytes | 19 (9.1 s) | 9.6 s | 199 / 214 / 114 |
| 15 s short, 1080p frames 140–159 (pan only), baseline | 1.42 s | 1.38 s | 59,101,184 bytes | 4 | — | 346 / 34 / 0 |
| 15 s short, 1080p frames 140–159 (pan only), ladder | 1.36 s | 1.34 s | 59,105,280 bytes | 4 | — | 346 / 34 / 0 |

Held-zoom and pan-only windows do the same work (the difference is noise). Memory does not grow
with frame count: the ladder binary peaked at 148,037,632 bytes over frames 48–67 and
149,676,032 bytes over frames 48–127, and the per-worker fitted cache stayed at or below its
67,108,864-byte budget in every run.

Visual difference against the baseline's exact rasters (lossless PNGs; mean and max absolute RGB
difference, PSNR, share of pixels off by more than 8, and the ratio of Sobel edge energy, ladder
over baseline, which drops when an image softens):

| Frame | Mean / max | PSNR | > 8 | Edge energy |
| --- | ---: | ---: | ---: | ---: |
| 1080p 10 (zoom held) | 0 / 0 | identical | 0% | 1.000 |
| 1080p 55, 72, 110 (dolly in) | 1.03–1.04 / 61–65 | 38.9–39.1 dB | 2.4–2.6% | 0.984–0.988 |
| 1080p 96 (deepest zoom) | 0.92 / 66 | 39.9 dB | 2.0% | 0.992 |
| 1080p 130 (reveal) | 1.17 / 71 | 38.0 dB | 3.0% | 0.983 |
| 4K 0 (follow, no zoom) | 0 / 0 | identical | 0% | 1.000 |
| 4K 72, 96 | 0.51–0.56 / 61–68 | 42.3–42.8 dB | 1.0–1.2% | 0.999–1.000 |
| 4K 143 | 0.72 / 75 | 40.1 dB | 1.7% | 0.997 |

Difference heat maps put every change on antialiased edges; 1:1 crops of thin railings, window
mullions, and lamp posts at 4K frame 96 show no visible softening or displacement. The largest
local changes are on the canopy's patterned rivets: resvg rasterizes a `<pattern>` tile at a
pixel-rounded size, so its phase already moves with the raster size (exact rasters at 1091 and
1001 px of `canopy.svg`, the first scaled to the second, place the rivets a row apart). The
baseline therefore re-phases the pattern on every zooming frame; the ladder re-phases it only at
level changes. A filter-width experiment (unwidened Catmull-Rom) changed PSNR by at most 0.1 dB, so
the remaining difference is the resampling itself, not over-filtering. `contain` and `cover` fits
would be placed at the level's rounded aspect ratio, so they stay exact (`tests/svg_zoom.rs`).
On a synthetic worst case (0.7-unit hairlines about 1 px wide at 1.5×) a zooming frame differs
by mean 1.34, max 74, 0.17 px centroid shift.

Determinism (ladder binary): the 1080p MP4 is identical at 1 and 4 threads and when assembled from
segment parts `4..6`, `0..2`, `2..4` rendered in that order by separate processes (`4c79e281…`).
Frames 0, 47, 55, 72, 96, 130, and 143 rendered as one 4-thread batch equal cold 1-thread renders
in reverse order. The 15-second short (pan-only cameras) is byte-identical to the baseline at
frames 0, 60, 123, 150, 238, 300, and 359, including both transitions.

Oversized fits: with the dawn sky moved inside the camera (`data-hf-parallax="0"`, the first 4K
draft), 4K frame 96 fails on the baseline with `rasterized image exceeds 64 MiB`. The ladder
binary renders it with one visible-region raster (the sky's on-screen part, uncached, re-rasterized
each frame it stays oversized). A region raster matches the same part of a whole raster except
where a path is clipped at the region edge: one 1/16 antialiasing step on a few pixels.
`svg::tests::a_region_matches_the_same_part_of_the_whole_raster` pins this: without the
clipped stroke the region matches within 1 everywhere (blur filter included); with it, every
pixel off by more than 1 lies under that stroke and differs by exactly one supersample step
(16 of 255; 4 to 10 pixels per fit). A region raster offset by 0.3 px fails the test.

### Temporal stability

Measured on lossless 1080p PNG sequences from both binaries. The temporal second difference
`|f(t+1) - 2 f(t) + f(t-1)|` stays small under smooth zoom and spikes on jumps, flicker, or shimmer.

| Sequence | Mean second difference, baseline → ladder | Worst frame |
| --- | ---: | --- |
| Rig demo without grain (scratch copy), frames 44–143 | 18.80 → 18.41 | ladder at most 1.2% above the baseline on any frame |
| `canopy.svg` alone, linear zoom 1.9× → 2.4× over 48 frames, rivet band | 2.54 → 1.90 | 2.98 → 2.49; ladder below the baseline on every frame |

- **Level switches.** The canopy probe crosses three ladder levels (frames 9→10, 26→27, 45→46).
  The frame-to-frame change rises to 1.62–1.69 there (0.9–1.3 between switches), inside the
  baseline's every-frame range of 1.08–1.68.
- **Sharpness.** Edge energy relative to the exact raster stays within 1.008–1.029 on all 48
  frames, with no sawtooth that follows the resample ratio.
- **Sub-pixel shift.** On pattern-free structure (the pillar), the best sub-pixel alignment
  between exact and ladder frames is within ±0.04 px on every sampled frame.
- **Rivet pattern.** The exact baseline re-phases the rivets relative to the struts by about 3 px
  on every zooming frame. The ladder keeps them locked within a level and re-phases once per
  level switch.
- **Reference check.** Against a 3× supersampled, box-downsampled reference of the same frames,
  both binaries score 34–41 dB and alternate which is closer. The rivet phase error belongs to
  resvg's pattern rasterization, not to the ladder; the ladder only makes it change less often.

## Native BT.709 output (2026-11-19)

The coastal film before the character artwork update, `examples/one-night/index.html` (1920×1080, 24 fps, 360 frames), `--size 1080p --fps 24
--threads 2 --quality high`, release build, desktop machine with 12 logical cores, measured
with `/usr/bin/time -l` (peak memory footprint and maximum resident set size) and
`AETHER_PROFILE=1`. The native encoder now converts frames itself (`src/encode/yuv.rs`,
fixed-point limited-range BT.709 with 2x2-averaged chroma) instead of OpenH264's built-in
limited-range BT.601 converter, and signals BT.709 in the SPS VUI.

| Build and film | Wall | User CPU | Peak footprint | Max RSS | `convert` (thread s) |
| --- | ---: | ---: | ---: | ---: | ---: |
| Before (BT.601), original film | 28.00 s | 48.41 s | 191,176,704 bytes | 374,329,344 bytes | 0.477 s (1.33 ms/frame) |
| After (BT.709), original film | 29.45 s | 50.34 s | 189,190,144 bytes | 373,493,760 bytes | — |
| After (BT.709), updated film (final) | 28.31 s | 49.20 s | 185,139,200 bytes | 391,880,704 bytes | 1.288 s (3.58 ms/frame) |

- **Conversion cost.** The integer converter costs 3.3–3.6 ms per 1080p frame against 1.33 ms
  for OpenH264's SIMD BT.601 path: about +0.8 thread seconds over 360 frames (under 2% of the
  export's CPU). Splitting the luma and chroma loops did not change it measurably.
- **Determinism.** The final export is byte-identical with `--threads 1`, `2`, and `4`, with
  `AETHER_SIMD=off`, and when assembled from two parts (`--segments 0..6` and `6..15`).
- **Stream.** `ffprobe` reports `color_range=tv`, `color_space=bt709`, `color_transfer=bt709`,
  `color_primaries=bt709` (before: `smpte170m` for all three). Saturated bars decode through
  FFmpeg to within 3 levels of the source RGB; reading the same stream as BT.601 moves pure red
  to 232 and adds 20 to the red channel of pure green.
- **Peak footprint variation.** Run-to-run peak footprint varied by about 6 MB for one input.
  A 0.25 s crossfade between the first two shots raised it to 230–240 MB (two full scenes
  composited at once), and a held zoom at the start of the night shot by about 30 MB (static
  zoomed rasters cached); neither was kept.

## Motion Scenes v1.1 scene reuse (2026-11-20)

Same machine, same session, release builds with LTO: the baseline is the binary built from the
tree before this change, the new one includes compiled-scene reuse, dependency lists, and
`data-composition-fit`. Each command ran under `nice -n 10`, alternating binaries, measured with
`/usr/bin/time -l`.

Fixture: a 16 s 1920x1080 film importing one 4 s middle scene 16 times; each middle scene imports
one 150-element leaf scene (text with keyframes, gradient boxes, SVG badges) 8 times: 144
instances and 19,617 flattened elements. Before, every instance re-read and re-parsed its file;
now each scene compiles once per set of values.

| Command | Baseline wall / user CPU / max RSS | v1.1 wall / user CPU / max RSS |
| --- | ---: | ---: |
| `check --safe-zone off --samples 1` (5 runs) | 0.92–0.96 s / 0.55–0.56 s / 114–115 MB | 0.54–0.56 s / 0.18 s / 111–112 MB |
| `bench --frame 120 --count 15 --threads 1` (3 runs) | 3.91–4.02 s / 3.82–3.87 s / 91 MB | 3.52–3.63 s / 3.45–3.52 s / 90 MB |

- **Compile.** User CPU for the compile-dominated `check` falls about 3x (0.55 s to 0.18 s). The
  `bench` wall time falls by the same ~0.4 s because it compiles first; its reported raster time
  (3.36–3.40 s against 3.35–3.46 s) is unchanged, as expected: rendering already shared decoded
  images across instances.
- **No-reuse film.** The full One Night export (`--size 1080p --fps 30 --threads 2`, one run
  each, three distinct scenes, so nothing is reused) took 32.56 s wall / 57.57 s user / 361 MB
  before and 33.36 s / 58.52 s / 363 MB after, within run-to-run noise, and the MP4 is
  byte-identical (2,295,659 bytes).
- **Determinism.** Tests check that reused instances compile to exactly the elements of separate
  copies and render the same frames, and that a fitted scene encodes byte-identically with one
  or two threads and from two assembled parts.

## Motion Scenes v1.2 shared scenes and occlusion (2026-11-20)

Same machine and session, release builds with LTO. The baseline binary was built from the tree
before this change (`git archive HEAD`); the new one adds shared copy-on-write element bodies,
shared text cache keys, occlusion of siblings under an opaque full-frame scene, and the
persistent watch cache. Each command ran under `nice -n 10`, alternating binaries; wall time,
user CPU, and max RSS were read from `getrusage` of the child process.

Fixture: `benchmarks/scene-reuse/` (added here; recreated from the v1.1 description, whose
fixture was not checked in): a 16 s 1920x1080 film importing a 4 s middle scene 16 times; each
middle scene imports a 152-element leaf scene (50 gradient cards, 50 SVG badges, 50 animated
labels) 8 times, staggered by 0.25 s: 144 instances and 19,617 flattened elements. The leaf root
has an opaque background, so about 21 overlapping leaves are active at frame 120 and all but the
newest are covered.

| Command | Baseline wall / user / max RSS | v1.2 wall / user / max RSS |
| --- | ---: | ---: |
| `check --safe-zone off --samples 1` (5 runs) | 0.13–0.31 s / 0.06–0.12 s / 87 MB | 0.08–0.09 s / 0.04–0.07 s / 61 MB |
| `bench --frame 0 --count 15 --threads 1` (3 runs) | 0.45–0.47 s / 0.42–0.43 s / 62–63 MB | 0.31–0.33 s / 0.30 s / 36 MB |
| `bench --frame 120 --count 15 --threads 1` (3 runs) | 5.56–5.72 s / 5.42–5.60 s / 65 MB | 1.21–1.40 s / 1.19–1.37 s / 36 MB |
| `bench --frame 240 --count 15 --threads 1` (3 runs) | 5.48–5.82 s / 5.39–5.69 s / 65 MB | 1.30–1.47 s / 1.28–1.44 s / 36 MB |
| `render --fps 30 --threads 4` (480 frames, 1 run) | 36.85 s / 140.37 s / 310 MB | 10.25 s / 38.64 s / 220 MB |

- **Memory.** Shared element bodies cut peak RSS by 26 MB for `check` and by 26–29 MB for
  `bench` (65 to 36 MB), independent of occlusion.
- **Raster time.** The 4.0–4.6x raster gain on overlapping frames comes from occlusion alone. With
  the leaf background removed (nothing can be occluded), `bench --frame 120 --count 15
  --threads 1` took 5.40–5.90 s before and 5.31–5.67 s after (3 runs each, within noise), at
  65 MB against 36 MB. Shared text cache keys did not change raster time here: the per-element
  cache (4,096 runs) already held every label of the active leaves; they avoid rasterizing each
  instance's text again and keep one copy in memory.
- **No-reuse film.** The One Night export (`--size 1080p --fps 30 --threads 2`, three distinct
  scenes, nothing reused or occluded: its crossfade keeps both scenes translucent) took
  52.26 / 54.60 / 56.25 s wall and 97.8 / 102.7 / 104.1 s user before, and 53.52 / 52.81 /
  53.28 s wall and 100.1 / 98.0 / 99.5 s user after, at 349–396 MB and 349–365 MB: within
  run-to-run noise.
- **Determinism.** One Night encodes byte-identically (3,325,340 bytes) with the baseline and,
  with the new binary, at `--threads 1`, `2`, and `4` and assembled from `--segments 0..6` and
  `6..15`. The reuse film encodes byte-identically with the baseline and, with the new binary, at
  one and four threads and from `--segments 0..5` and `5..16`. Tests compare occluded frames
  with the same frames drawn in full (fades, translucent roots, pushes, `contain` and `cover`
  fits, moving and filtered hosts, backdrop blur and blend modes inside the cover, and a
  fractional `--scale`).

## 2.5D spaces (2026-11-20)

Release builds (`cargo build --release`) of the commit before this change and of the change, on
one generic 4-core Linux container. Each command ran under `nice -n 10`, alternating binaries;
wall time, user CPU, and max RSS come from `getrusage` of the child process.

**Existing 2D compositions.** No 2D drawing path changed; elements outside a space only gain zero
2.5D state, and a renderer without a `data-hf-space` builds no space index.

| Command (3 runs) | Baseline wall / user / max RSS | 2.5D wall / user / max RSS |
| --- | ---: | ---: |
| `bench examples/premium-promo.html --frame 0 --count 20 --threads 1` | 0.88–0.95 s / 0.85–0.92 s / 52–53 MB | 0.91–0.95 s / 0.87–0.89 s / 53 MB |
| `bench examples/cinematic-camera-rig-6s.html --frame 60 --count 20 --threads 1` | 10.36–10.69 s / 10.22–10.48 s / 149–152 MB | 9.92–10.16 s / 9.85–9.98 s / 142–149 MB |
| `bench examples/cinematic-dystopia.html --frame 30 --count 20 --threads 1` | 1.96–2.12 s / 1.92–2.08 s / 63 MB | 1.96–2.31 s / 1.93–2.25 s / 63–64 MB |
| `check examples/one-night/index.html --safe-zone off --samples 1` | 0.14–0.15 s / 0.12–0.14 s / 26 MB | 0.14–0.22 s / 0.13–0.21 s / 26 MB |
| `render examples/cinematic-camera-rig-6s.html --fps 30 --threads 4` (1 run) | 27.04 s / 77.55 s / 698 MB | 26.59 s / 75.90 s / 692 MB |

All differences are within run-to-run noise. The camera-rig MP4 is byte-identical to the baseline,
170 PNG stills (frames 0, 1/3, 1/2, 2/3, and last at 30 fps of all 34 other examples) are
byte-identical, and `check` prints identical JSON for all 33 top-level examples.

**The 2.5D example.** `examples/depth-lantern-valley.html` (1920x1080, 180 frames, 16 planes; one
tilted lake split by the BSP order where the ridges and reeds stand on it):

| Command | Wall / user / max RSS |
| --- | ---: |
| `render --fps 30 --threads 1` | 75.82 s / 73.55 s / 299 MB |
| `render --fps 30 --threads 4` | 27.72 s / 76.34 s / 798 MB |
| `bench --frame 60 --count 15 --threads 1` | 4.59 s raster (306 ms per frame) |

`AETHER_PROFILE=1` attributes 67 ms per frame to drawing the lake's layer (`plane-layer`, two
passes of a four-stop gradient over about 1.5 times its on-screen area) and 94 ms to warping its
four fragments (`plane-warp`, trilinear where minified); everything else costs what it did with
three separate lake strips (152 ms). Sizing the layer by the plane's on-screen area instead of the
frame cut the frame from 375 to 306 ms, and drawing rows only across each fragment's projected
outline keeps the warp proportional to the visible area. The one- and four-thread exports are
byte-identical; `tests/space.rs` also checks reverse-order frames and a segmented export
assembled from part files against the one-thread MP4.

## 0.2.0: native low-poly 3D (2026-11-21)

Release builds of the commit before this change (baseline) and of the change, on one generic 4-core
Linux cloud container. Wall time, user CPU, and peak RSS come from `getrusage` of the child process;
`bench` numbers are the raster time it prints; `AETHER_PROFILE=1` gives the `scene-3d` phase.

**Existing 2D and 2.5D compositions.** No existing drawing path changed: a scene adds one
`Content` variant, one `FrameState` field (0 for every other element), and an empty per-renderer
workspace that allocates nothing until a scene is drawn.

| `bench --frame 30 --count 20 --size 1080p` (2 runs) | Baseline | 0.2.0 |
| --- | ---: | ---: |
| `examples/premium-promo.html` | 0.803 / 0.873 s | 0.868 / 0.855 s |
| `examples/depth-lantern-valley.html` | 4.994 / 4.925 s | 4.949 / 4.941 s |
| `examples/cinematic-camera-rig-6s.html` | 3.989 / 4.121 s | 3.900 / 4.040 s |

All differences are within run-to-run noise. 96 PNG stills (frames 0, 37, and 95 at `--scale 0.5`
of every top-level example) and four MP4 exports (`depth-lantern-valley`, `premium-promo`,
`transitions`, `layer-motion-blur` at `--scale 0.5 --threads 2`) are byte-identical to the baseline.
`check` output gains an empty `scenes3d` array.

**Lighthouse Isle before the volumetric shaft.** `examples/lighthouse-isle-3d.html` (1920x1080, 300 frames at 30 fps, 2,594
triangles, `data-hf-3d-antialias="3"`, bloom on the viewport and the sun, a moving sky gradient):

| `render --size 1080p --priority normal` | Wall / user / peak RSS |
| --- | ---: |
| `--threads 1` | 68.6 s / 65.2 s / 113 MiB |
| `--threads 2` | 34.8 s / 65.9 s / 220 MiB |
| `--threads 4` | 20.3 s / 65.8 s / 366 MiB |
| same composition without geometry, `--threads 1` | 32.8 s / 31.5 s / 121 MiB |
| same composition without geometry, `--threads 4` | 10.2 s / 31.9 s / 345 MiB |

The one-, two-, and four-thread MP4s are byte-identical (1.8 MB). Total CPU does not grow with
threads. Peak RSS grows by about 85 MiB per worker with or without geometry: that is the existing
2D renderer (frame, bloom effect layers, gradient caches). The 3D path adds about 5 MiB per worker
at 1080p: the triangle list, one 64 px tile of samples (at most 512 KiB), and the one-entry memo of
the resolved region (8 MiB at 1080p, capped at 64 MiB). At 4K (`bench --count 10 --size 4k`), peak
RSS is 185 MiB with geometry and 182 MiB without.

**Cost by supersampling** (`scene-3d` per frame, one thread, the example without bloom so each
frame draws the scene once):

| `data-hf-3d-antialias` | 1080p | 4K |
| --- | ---: | ---: |
| 1 | 26.8 ms | 98.6 ms |
| 2 (default) | 37.0 ms | 150.7 ms |
| 3 | 61.2 ms | 243.6 ms |
| 4 | 95.1 ms | 340.2 ms |

**Optimizations, profiled first** (`bench --frame 150 --count 15 --size 1080p --threads 1`, with
bloom, so the viewport is an effect layer drawn on black and on white):

| Step | `scene-3d` per frame | Frame |
| --- | ---: | ---: |
| First version: per-sample edge tests over each triangle's bounding box, two full rasterizations | 207 ms | 369 ms |
| Exact span solver per sample row (integer ceil/floor of each edge inequality) and a one-entry memo that replays an identical draw | 92 ms | 261 ms |
| Coverage read from depth (no color clear) and an opaque fast path when compositing | 62 ms | 230 ms |

Every step keeps output byte-identical: the span solver is tested against the per-sample loop on 400
random triangles, the memo replays the rows it recorded, a sample is covered exactly when its
reversed depth is above 0, and the fast path writes what `blend_pixel` writes at full coverage. The
stills of the example were compared before and after each step.

**GPU backend.** `--features gpu` on Mesa lavapipe (Vulkan implemented on the CPU, so these are not
hardware GPU timings): 56 ms per `scene-3d` call at 1080p including readback. Against the CPU, at
most 20 of 2,073,600 pixels of the example differ by more than 1% (peak 24 levels, all on
triangle edges); `tests/scene3d_gpu.rs` measured a mean difference of 0.0004 levels or less and at
most 6 of 129,600 pixels more than 2 levels apart at 480x270. Without an adapter the GPU build falls
back to the CPU, byte-identical to the CPU build.

## Procedural shaders

[`examples/shader-showcase.html`](examples/shader-showcase.html) at 1920x1080, `bench --count 15
--threads 1` from three starting frames. `shader-eval` runs the compiled program, and `shader-blend`
resamples the grid and composites it. Frame 30 has the intro backdrop at quarter resolution and the ring
at full resolution, frame 165 has the aurora at half resolution and the lower third, and frame 300 has
the grid at half resolution and the call to action.

| Start | `shader-eval` per call | `shader-blend` per call, before | after | Frame, before | after |
| --- | ---: | ---: | ---: | ---: | ---: |
| 30 | 16.8 ms | 23.2 ms | 7.7 ms | 118.8 ms | 73.0 ms |
| 165 | 54.7 ms | 24.0 ms | 8.4 ms | 188.7 ms | 141.6 ms |
| 300 | 17.2 ms | 70.7 ms | 26.4 ms | 103.8 ms | 61.7 ms |

The first version sampled and blended pixel by pixel with branches and per-pixel bilinear lookups.
The second interpolates each grid row horizontally once (two rows are kept) and runs the vertical
interpolation and the blend as branch-free row kernels (`lerp_rows`, `shader_blend_row` in
`paint/kernels.rs`, AVX2 at run time). The arithmetic per pixel is unchanged. All seven stills of the
example were byte-identical before and after, and identical with `AETHER_SIMD=off` (63.8 ms per
frame from frame 300, against 61.7 ms). The baseline AVX2 evaluator gives the same values as the
scalar one: `shader::tests::simd_build_matches_the_baseline` compares them bit for bit. Stills of
nine existing examples (three frames each) were byte-identical to the build before shaders.

