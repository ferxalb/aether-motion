# AETHER Motion patch of oxideav-aac 0.1.7

This directory is `oxideav-aac` 0.1.7 (MIT, see `LICENSE`) with its `src/` unchanged except for one
function, plus a trimmed `Cargo.toml` that drops the upstream integration tests (they are not vendored).

- `src/filterbank.rs`, `forward_mdct`: the upstream version evaluates one `cos` per input sample and
  output coefficient, about 2.1 million per channel and frame, which made AAC encoding slower than real
  time. The patched version uses a cached table of `cos(2*pi*j / 4N)` indexed with exact integer
  arithmetic (the cosine argument is `2*pi*M / 4N` for an integer `M`). The coefficients agree with the
  upstream formula to a relative 1e-11 or better (floating-point rounding); `aether_patch_tests` checks it.

Remove this directory and the `[patch.crates-io]` entry in the root `Cargo.toml` to go back to upstream.
