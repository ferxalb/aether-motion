# AETHER Motion patch of openh264 0.9.10

This directory is the `openh264` 0.9.10 wrapper crate (BSD-2-Clause, see `LICENSE`; the OpenH264
codec itself still comes unmodified from `openh264-sys2`) with its `src/` unchanged except for one
option, plus a trimmed `Cargo.toml` that drops the upstream examples, tests, and benches (they are
not vendored). The unit tests inside `src/` are upstream's and are not run here: they need the
`rand` dev-dependency and the H.264 fixtures under `tests/data`, which were left out with the rest.
`rust-version` is raised from upstream's 1.85 to 1.89, the minimum of the locked `wide` 1.7.1.

- `src/encoder.rs`, `EncoderConfig::base_qp`: with `RateControlMode::Off`, OpenH264 encodes every
  frame at the spatial layer's `iDLayerQp`, which upstream never sets, so it is always 26 and the
  `qp` range only bounds the adaptive-quantization adjustment of P frames. The encoder is
  initialized lazily inside the first `encode_at`, so the value cannot be set from outside before
  the first frame. The patch adds `base_qp(u8)` and writes it to `iDLayerQp` in `reinit`, which is
  what AETHER Motion's `--quality` presets use (`src/encode.rs`).

To go back to upstream, remove this directory, the `openh264` entry under `[patch.crates-io]` in the
root `Cargo.toml`, and the `.base_qp(quality.base_qp())` call in `make_encoder` (`src/encode.rs`),
which upstream does not have. Without that call every preset encodes at upstream's QP 26, so
`--quality` only changes the adaptive-quantization range and `standard` is the only preset that
behaves as documented.
