# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

`AGENTS.md` is the authoritative contributor guide: it holds the full supported HTML/CSS contract, the language policy (all prose in English), the CPU budget rules, and the scene design standard. Read it before changing code or compositions. Do not duplicate it here.

## What this is

AETHER Motion is a native Rust renderer (no browser, no JS) for a documented subset of HyperFrames HTML. A composition is HTML with `data-hf-*` attributes. Output must be deterministic: a frame is a pure function of time. Unsupported features must fail with an explicit error, never render something wrong.

## Commands

```sh
cargo fmt --check
cargo clippy --all-targets --locked -- -D warnings   # CI gate
cargo test --locked                                  # CI gate (ci.yml on Linux/macOS/Windows)
cargo test --release <name>                          # one targeted test; test files live in tests/*.rs
nice -n 15 cargo build --release -j 1                # release build (LTO, over 10 minutes)

target/release/aether-motion --help                  # commands grouped by task
target/release/aether-motion check <file.html>       # JSON report; iterate until ok: true
target/release/aether-motion render <file.html> --output frame.png --frame N
target/release/aether-motion render <file.html> --output video.mp4
target/release/aether-motion bench <file.html> --frame START --count 15 --threads 1
```

- Do not build or run `cargo test` to validate docs, skills, examples, compositions, or assets. Use the existing `target/release` binary and `check`/`render`. Building needs user approval (state why and the expected duration). Never run the whole test suite repeatedly.
- `AETHER_PROFILE=1` prints per-phase thread time. `AETHER_SIMD=off` forces the scalar kernels to confirm that output is identical. `AETHER_3D_BACKEND` and `--3d-backend cpu|gpu|cuda|auto` select the 3D backend. The CPU backend is the deterministic reference, and `cuda` needs `--features cuda`.
- Keep `src/cli/help/catalog.rs` in step with the flags each command parses.
- Any new file under `skills/` must be listed in `src/setup/skills.rs` (embedded and installed by `setup`).

## Architecture

The pipeline runs in this order: `parse` (HTML/CSS into `model.rs`) → `render` (layout, then paint per frame) → `encode` (native H.264 and AAC into MP4). `docs/architecture.md` has the detailed module-by-module layout and resource limits. Update it, and each file's `//!` header comment, when you add or move modules.

- `src/model.rs` is the parsed composition: styles and time-sampled motion tracks. Motion is compiled to tracks at parse time (CSS animation, `data-hf-keyframes`, camera, motion paths, split text), so rendering only samples them at `t`.
- `src/parse/` is the HTML/CSS front end (`kuchiki`). `mod.rs` is the `parse_file` facade, with submodules for `css`, `values`, `style`, `animation`, `split`, `scenes`, `variables`, and others. Compositions with `data-hf-split-wrap` are parsed twice (the first parse is measured, the second builds the wrapped rows).
- `src/render/` has `layout/`, `paint/` (rasterizer; hot per-row loops in `paint/kernels.rs` are scalar, branch-free, and also AVX2-compiled with runtime dispatch, so they must stay bit-identical), `text/`, and the effect-layer, tile-cache, and frame-state machinery.
- Feature domains with their own modules: `camera/` (rig, shots, parallax), `space/` (2.5D planes), `scene3d/` (native low-poly 3D with CPU, wgpu, and CUDA backends; see `docs/3d.md`), `audio/` (decode, effects, mix, AAC mux), `video/` (H.264 decode), `vector/` (SVG via resvg), `captions/`, `blocks/` (ready-made HTML blocks and themes), and `speed.rs`.
- `src/cli/` implements commands (`render`, `check`, `preview`, `bench`, `block`, `init`, `add`, `doctor`, `captions`, ...). `src/agent/mcp/` is the MCP stdio server (`aether_guide`, `aether_write`, `aether_block`, `aether_check`, `aether_snapshot`, `aether_render`). `src/setup/` installs the bundled skills and the MCP config into a user's project.
- `vendor/` holds patched crates (`oxideav-aac`, `openh264`) wired in through `[patch.crates-io]`. See their `AETHER-PATCH.md` files.
- Other directories: `examples/` (reference compositions), `blocks/`, `templates/`, `skills/` (Agent Skills; update them after a substantial user-facing change), `npm/` (the `@aether-motion/cli` shim), `benchmarks/` and `BENCHMARK.md`, `limits.md`, and `docs/releasing.md` (tag `v<version>` must match `Cargo.toml`; update `CHANGELOG.md`).

## Conventions that bite

- Keep `src/parse.rs`-style facades small. Extract cohesive submodules rather than growing `parse`/`render` catch-alls.
- Every Rust file starts with a `//!` comment. Keep it accurate when responsibilities change.
- Speedups must keep output byte-identical, except the documented reduced-resolution `backdrop-filter` blur path (error of 2 levels or less). Profile first and record before/after numbers in `BENCHMARK.md`.
- Never put machine-identifying details (hardware models, OS builds, hostnames, local paths) in tracked docs or benchmarks. Keep the measured numbers.
- New scenes and examples follow the `skills/aether-motion-cinematic` standard and are checked by looking at rendered frames, not just `check`.
