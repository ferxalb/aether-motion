<h1 align="center"><img src="assets/readme/hero.jpg" alt="AETHER Motion" width="800"></h1>

<p align="center"><b>Write HTML. Render video. No browser.</b></p>

<p align="center">
  <a href="https://github.com/ferxalb/aether-motion/actions/workflows/ci.yml"><img src="https://github.com/ferxalb/aether-motion/actions/workflows/ci.yml/badge.svg" alt="CI"></a>
  <a href="https://github.com/ferxalb/aether-motion/releases"><img src="https://img.shields.io/github/v/release/ferxalb/aether-motion?style=flat" alt="Release"></a>
  <a href="https://skills.sh/ferxalb/aether-motion"><img src="https://skills.sh/b/ferxalb/aether-motion" alt="skills.sh installs"></a>
  <a href="LICENSE"><img src="https://img.shields.io/badge/license-Apache%202.0-blue.svg" alt="License"></a>
  <img src="https://img.shields.io/badge/built%20with-Rust-orange?logo=rust" alt="Rust">
  <img src="https://img.shields.io/badge/platforms-macOS%20%7C%20Linux%20%7C%20Windows-lightgrey" alt="Platforms">
</p>

<p align="center">
  <a href="#quick-start">Quick Start</a> |
  <a href="#blocks-and-themes">Blocks</a> |
  <a href="examples">Examples</a> |
  <a href="AGENTS.md">Composition Reference</a> |
  <a href="#agent-skills">Agent Skills</a> |
  <a href="https://github.com/ferxalb/aether-motion/releases">Download</a>
</p>

AETHER Motion is a free, open-source video renderer that turns HTML and CSS compositions into deterministic MP4 videos. It is a single native binary: no headless Chrome, no Node.js, no FFmpeg. It reads the HyperFrames composition format (clips, tracks, and timing as data attributes) and adds native motion: keyframes, camera moves, kinetic type, scene transitions, and glass.

Using a coding agent? Connect the local MCP server for direct tool access, or install the AETHER Motion Agent Skills for reusable composition, rendering, SVG, and import workflows.

## Quick Start

```bash
npx @aether-motion/cli init my-video    # or: pnpm dlx / bunx / yarn dlx (no install needed)
```

With the binary on your `PATH` (`npm i -g @aether-motion/cli`, a [release download](https://github.com/ferxalb/aether-motion/releases), or `cargo install`), the commands are short:

```bash
aether-motion init my-video   # sample video + agent skills + MCP config, ready to go
cd my-video
aether-motion preview         # render a frame and open it (add --watch to refresh on save)
aether-motion render          # export out/index.mp4
```

`init` writes `aether.json`, a sample `index.html`, project `AGENTS.md`, the bundled Agent Skills (`.claude/skills`, `.agents/skills`), and the MCP server entry for Claude Code (`.mcp.json`; Cursor and VS Code too when their folders exist or with `--all`). It never overwrites existing files without `--force`, merges into existing MCP files, and is safe to re-run. Other commands: `check`, `add [block]`, `agent setup|status`, `doctor`.

### With an AI coding agent

Install AETHER Motion (see [Install](#install)), then register its MCP server. For Claude Code:

```bash
claude mcp add aether-motion -- aether-motion mcp
```

For other agents, add `aether-motion mcp` as a stdio MCP server in their settings.

The server supports MCP `2026-07-28` and legacy clients that begin with `initialize`.

Try a prompt like:

> Using the AETHER Motion tools, make a 12-second 1080p product intro with a kinetic title, three glass feature cards, an animated counter, and a push transition between scenes.

The agent reads the contract with `aether_guide`, writes the file, validates it with `aether_check` (which returns the exact error and a fix), looks at any moment with `aether_snapshot`, and exports with `aether_render`.

### Manually with the CLI

```bash
aether-motion check  examples/premium-promo.html                                       # validate
aether-motion render examples/premium-promo.html --output frame.png --frame 90         # preview a frame
aether-motion render examples/premium-promo.html --output promo.mp4 --size 1080p --fps 30
```

**Requirements:** none. FFmpeg is optional (`--encoder ffmpeg`).

## Install

Download the binary for your system from [Releases](https://github.com/ferxalb/aether-motion/releases) and put `aether-motion` on your `PATH`. Or build it with Rust:

```bash
cargo install --git https://github.com/ferxalb/aether-motion
```

Earlier release archives use the executable name `aether-hyperframes`. From a local checkout, run `cargo install --path . --locked` to install `aether-motion` before the next release.

## What You Can Build

- Product launch videos and feature announcements
- Metric and milestone videos with animated counters and stat cards
- Kinetic title cards and lower thirds
- Vertical (9:16) social videos at 1080p or 4K
- Automated video in CI and content pipelines, with output identical on every run

## How It Works

Define a video as HTML. Data attributes set timing and tracks; `data-hf-*` attributes and CSS `@keyframes` add motion.

```html
<div id="stage" data-composition-id="launch" data-width="1920" data-height="1080"
     data-duration="6" style="position: relative; background: #0b1020">

  <div id="intro" class="clip" data-start="0" data-duration="3" data-track-index="0"
       style="position: absolute; left: 0; top: 0; width: 100%; height: 100%">
    <div data-hf-camera='[{"at":0,"zoom":1},{"at":3,"zoom":1.15,"easing":"ease-out-expo"}]'
         style="position: absolute; left: 0; top: 0; width: 100%; height: 100%;
                display: flex; align-items: center; justify-content: center">
      <h1 data-hf-split="words" data-hf-split-from="up"
          style="color: white; font-family: sans-serif; font-size: 120px">Launch day</h1>
    </div>
  </div>

  <div id="outro" class="clip" data-start="intro-0.6" data-duration="3.6" data-track-index="1"
       data-hf-transition="push-left" data-hf-transition-from="intro"
       style="position: absolute; left: 0; top: 0; width: 100%; height: 100%; background: #12203d;
              display: flex; align-items: center; justify-content: center">
    <div data-hf-counter="12400" data-hf-counter-suffix="+"
         style="color: #4ee0b5; font-family: sans-serif; font-size: 160px; font-weight: bold"></div>
  </div>
</div>
```

AETHER Motion parses the composition, lays it out, and draws every frame with its own software renderer, then encodes H.264 with a built-in encoder. Each frame is a function of time only, so the same input produces the same video on every run. Anything outside the supported subset fails with an explicit error instead of rendering something wrong.

<p align="center">
  <img src="assets/readme/motion-preview.gif" alt="A rotating ASCII planet, kinetic typography, and a sequence of reveals on dark feature cards" width="640">
</p>
<p align="center">
  Rendered with AETHER Motion from the <a href="examples/aether-monochrome-promo.html">monochrome promo composition</a>.
</p>

## Blocks and Themes

Ready-made, parametrized pieces, printed as editable HTML:

```bash
aether-motion block list
aether-motion block hero-title --set title="Launch day" --set id=intro
aether-motion block theme-midnight
```

| Block | What it is |
| --- | --- |
| `hero-title` | Full-screen hero: kicker, word-by-word headline, subtitle |
| `feature-card` | Glass card with accent dot, title, and body |
| `stat-card` | Big value that pops in letter by letter, with a label |
| `counter-card` | Number that counts up, with prefix or suffix and a label |
| `cta` | Closing headline, gradient button, and URL |
| `glow` | Two soft, slowly drifting background glows |
| `theme-midnight`, `theme-sunrise`, `theme-paper` | Recolor every block at once |

[`examples/premium-promo.html`](examples/premium-promo.html) is a 15-second promo built from blocks.

## Why AETHER Motion?

- **One binary:** no browser, no Node.js, no FFmpeg. Runs on macOS, Linux, and Windows, including CI runners and servers.
- **Deterministic:** same input, same frames, same output. Generic fonts use an embedded Roboto, so text matches across machines.
- **Honest errors:** unsupported HTML or CSS returns the exact problem and a suggested fix, never a silently wrong video.
- **Native motion:** keyframes with expo, back, and `cubic-bezier` easing, CSS `@keyframes`, camera zoom and pan, kinetic text, reveals, counters, and crossfade, push, cover, wipe, and zoom transitions.
- **Captions:** `data-hf-captions` turns a transcript (`transcript.json`, OpenAI Whisper or whisper.cpp JSON, SRT, VTT) into word-highlighted captions; `aether-motion captions` converts transcripts to SRT or VTT. See `examples/captions.html`.
- **Video clips:** local H.264 MP4 `<video>` (its sound included) with trim (`data-media-start`), loop, `object-fit`, and the usual fades, transforms, and clipping; see `examples/video.html`.
- **Sample-accurate audio:** `<audio>` clips (WAV, FLAC, Ogg Vorbis, MP3) with volume, fades, trim, and loop are mixed and encoded to AAC with no FFmpeg; see `examples/audio.html`.
- **Polished look:** flexbox, text wrapping, gradients, layered and inset shadows, blur, frosted glass, rotation, vignette, and film grain.
- **Easy on your machine:** renders at low priority on a quarter of your cores by default. `--threads N` goes faster; `--scale 0.5` gives quick drafts.
- **Top quality by default:** MP4 exports use `--quality max` (H.264 quantizer 18) for sharp text, clean gradients, and visible grain; `--quality draft|standard|high` trade fidelity for smaller files at about the same CPU cost.
- **Fast:** the 15-second premium promo (450 frames at 1080p/30) renders to MP4 in about 15 seconds with default settings on a laptop. See [BENCHMARK.md](BENCHMARK.md).
- **Open source:** Apache 2.0.

## AETHER Motion vs HyperFrames and Remotion

AETHER Motion uses the [HyperFrames](https://github.com/heygen-com/hyperframes) composition format but replaces its browser pipeline with a native renderer. Remotion takes a React-first, frame-driven approach. Choose based on the source format, runtime, and agent workflow you need:

| | **AETHER Motion** | **HyperFrames** | **Remotion** |
| --- | --- | --- | --- |
| Authoring model | HTML composition with clips and native `data-hf-*` attributes | HTML compositions with browser-based JavaScript timelines | React components and frame-indexed composition code |
| Rendering | Native Rust renderer; no browser, Node.js, or FFmpeg required | Browser rendering through the CLI, plus hosted cloud rendering | Browser-based video rendering through the CLI; Player and Studio provide interactive previews |
| Runtime and scope | One binary; documented HTML/CSS subset, with explicit errors outside it | Node.js 22+, Chrome, and FFmpeg for the local CLI; full browser HTML/CSS/JS | Node.js and a React project; supports frame-driven components and browser media |
| Motion | Native keyframes, camera, kinetic type, transitions, and limited CSS `@keyframes` | GSAP, CSS, Lottie, Three.js, WAAPI, and browser adapters | `useCurrentFrame()`, `interpolate()`, `spring()`, React components, and transition primitives |
| Audio and video | Local `<audio>` mixed sample-accurately into AAC; static images/SVG; local H.264 MP4 `<video>` (no other codecs) | Browser audio/video and other media elements | `<Audio>`, `<Video>`, images, and other React media components |
| AI-agent integration | [Local stdio MCP server](#with-an-ai-coding-agent) **and** [Agent Skills](#agent-skills) | [Hosted MCP](https://hyperframes.heygen.com/tools/mcp-server-for-video); [CLI and Agent Skills](https://github.com/heygen-com/hyperframes#skills) for coding agents | [Agent Skills](https://www.remotion.dev/docs/ai/skills) and [Studio WebMCP](https://www.remotion.dev/docs/ai/webmcp) (agent support varies) |
| Best fit | Reproducible native rendering with a small, explicit contract | Browser-powered HTML video projects and hosted agent rendering | Programmatic video built from React components and frame-based logic |

Use AETHER Motion when you want a lightweight native renderer and its supported HTML contract. Use HyperFrames when you need browser features, other video formats, or its hosted MCP workflow. Use Remotion when the project is React-based or needs frame-driven React components. The agent integrations differ: AETHER Motion combines local MCP with installable skills, HyperFrames offers hosted MCP plus CLI skills, and Remotion offers Agent Skills plus Studio WebMCP.

## Output

| Composition | `--size 1080p` | `--size 4k` |
| --- | --- | --- |
| Landscape (16:9) | 1920 × 1080 | 3840 × 2160 |
| Vertical (9:16) | 1080 × 1920 | 2160 × 3840 |

Up to 60 fps, H.264 MP4 (yuv420p, constant frame rate), or a single PNG frame with `--frame N`.

## Documentation

- [Composition reference](AGENTS.md): every supported attribute, CSS property, and motion feature
- [Limits](limits.md): what is not supported
- [Examples](examples): ready-to-render compositions
- [Architecture](docs/architecture.md): source layout, memory limits, encoding
- [Benchmarks](BENCHMARK.md): measurements and method
- [Changelog](CHANGELOG.md): releases (SemVer; while `0.x`, minor releases may break the contract, and every break is listed)

## Agent skills

Install the skill collection with `npx skills add ferxalb/aether-motion`, then select the workflows you want to add to your agent. The MCP server and Agent Skills are complementary: MCP exposes AETHER Motion tools, while the skills teach an agent how to use the composition contract and workflows.

| Skill | Use it for |
| --- | --- |
| `aether-motion` | Creating and editing compositions |
| `aether-motion-cli` | Checking, inspecting, and rendering compositions |
| `aether-motion-blocks` | Finding and adapting ready-made blocks and themes |
| `aether-motion-svg` | Creating and placing static SVG artwork |
| `aether-motion-import` | Porting existing HyperFrames or Remotion projects |

## Contributing

```bash
cargo build --release
cargo test --release <test-name>
```

CI builds and tests on Linux, macOS, and Windows. Read [AGENTS.md](AGENTS.md) for code organization and CPU budget rules before profiling or benchmarking.

- Bugs and feature requests: [GitHub Issues](https://github.com/ferxalb/aether-motion/issues)

## License

[Apache 2.0](LICENSE). Includes Roboto (Apache 2.0). Audio uses Symphonia (MPL-2.0) for decoding and `oxideav-aac` (MIT) for AAC encoding; AAC and MP3 are patent-licensed formats, so check your distribution needs.
