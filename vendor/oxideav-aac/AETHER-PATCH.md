# AETHER Motion patch of oxideav-aac 0.1.7

This directory is `oxideav-aac` 0.1.7 (MIT, see `LICENSE`) with its `src/` unchanged except for one
function, plus a trimmed `Cargo.toml` that drops the upstream integration tests (they are not vendored).

- `src/filterbank.rs`, `forward_mdct`: the upstream version evaluates one `cos` per input sample and
  output coefficient, about 2.1 million per channel and frame, which made AAC encoding slower than real
  time. The patched version folds the `N` windowed samples into a DCT-IV of length `N/2` and evaluates
  it with one `N/4`-point radix-2 complex FFT (twiddles and bit reversal cached per thread and length),
  O(N log N), for every power-of-two length (all AAC-LC transforms: 2048 and 256). Other lengths use a
  cached table of `cos(2*pi*j / 4N)` indexed with exact integer arithmetic (the cosine argument is
  `2*pi*M / 4N` for an integer `M`). Both agree with the upstream formula to a relative 1e-11 or
  better (floating-point rounding); `aether_patch_tests` checks lengths 8 to 2048 and 960. Encoding 60 s
  of stereo audio went from about 96 s (upstream) and 16.7 s (table) to 1.4 s, and the encoded
  bytes of the measured files did not change (see `BENCHMARK.md`).

Remove this directory and the `[patch.crates-io]` entry in the root `Cargo.toml` to go back to upstream.
