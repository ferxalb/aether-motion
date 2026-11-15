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
