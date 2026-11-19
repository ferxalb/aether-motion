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

### Cinematic compositing

[The Morning Shift](examples/cinematic-dystopia.html) layers original SVG architecture, animated fog and rain, and a 12 FPS transparent character sequence. The native camera pans across independently weighted depth planes, and localized bloom and root filmic controls finish the shot.

```bash
aether-motion check examples/cinematic-dystopia.html
aether-motion render examples/cinematic-dystopia.html --output out/cinematic-4k.png --size 4k --fps 24 --frame 72
```

[The Morning Shift — Into the Shadows](examples/cinematic-walk-15s.html) is a 15-second
short with a wide city view, a loading-dock side elevation, and an oblique view of the
lamp yard. The cameras pan at fixed scale. The 12 FPS gait pairs planted-sole pivots
and contact shadows with stepped motion. The worker passes through stationary light
masks and behind the overpass pillar. His reflected cels are masked to the wet pavement.
The shot changes are hidden rather than cut: a near-lens steel column carries a linear
`wipe-left` at 5 seconds, and a lamp-lit fog veil covers a short crossfade at 10 seconds.
Every shot keeps the shared walk phase.
The included [cel drawing script](examples/cinematic-walk-assets/draw-cels.py) regenerates
PNGs on macOS, and the [set drawing script](examples/cinematic-walk-assets/draw-sets.py)
regenerates the SVG plates.

```bash
aether-motion check examples/cinematic-walk-15s.html
aether-motion render examples/cinematic-walk-15s.html --output out/cinematic-walk-1080p-v3.mp4 --size 1080p --fps 24 --quality high --threads 2
# The validated 3840x2160 master (360 frames; --threads changes speed, not output)
aether-motion render examples/cinematic-walk-15s.html --output out/cinematic-walk-4k-v3.mp4 --size 4k --fps 24 --quality max --threads 4
```

### Camera rig and shot composer

A camera container can follow a character and move through a sequence of shots on one
continuous camera. `data-hf-camera-subject` names the element to follow and
`data-hf-camera-anchor` sets where it sits in the frame; `data-hf-shots` lists presets
(`static`, `follow`, `pan`, `dolly-in`, `dolly-out`, `reveal`) played back to back:

```html
<section id="rig" data-hf-camera-subject="worker" data-hf-camera-anchor="0.4 0.6"
    data-hf-camera-lag="0.45" data-hf-camera-lookahead="0.45"
    data-hf-shots='[{"preset":"follow","duration":2,"from":{"zoom":1.35}},
                    {"preset":"dolly-in","duration":2,"zoom":1.6},
                    {"preset":"reveal","zoom":1.0,"to":[0,40]}]'>
  <img class="plate" src="sky.svg" data-hf-parallax="0.3">
  <div id="worker" data-hf-parallax="1.14">...</div>
</section>
```

Shots use the `smooth` easing by default: zoom, focus, framing, and follow weight are cubic
curves with automatic tangents, so velocity stays continuous across shots and the camera never
overshoots a key. Classic `data-hf-camera` keyframes take `"easing":"smooth"`, `anchorX`,
`anchorY`, and `follow` too. The camera pose is a pure function of time. The only stateful
part of tracking, the dead zone and the lag, is baked once at load on a fixed 240 Hz grid.
Frames therefore match in any order, at any thread count, and across segmented renders.
Cameras that use no rig feature keep their exact classic tracks. Scene changes remain clips
joined by `data-hf-transition`; shots never interpolate between unrelated scenes. The
[six-second rig demo](examples/cinematic-camera-rig-6s.html) tracks the walker of The Morning
Shift, dollies in, and reveals the skyline. It reuses the short's artwork and leaves the
short unchanged.

```bash
aether-motion check examples/cinematic-camera-rig-6s.html      # lists the resolved keys under "cameras"
aether-motion render examples/cinematic-camera-rig-6s.html --output out/camera-rig-1080p.mp4 --size 1080p --fps 24
aether-motion render examples/cinematic-camera-rig-6s.html --output out/camera-rig-4k.png --size 4k --fps 24 --frame 96
```

Known limits: the subject is followed by translation (scale and rotation of the subject and its
ancestors are not followed), rig cameras cannot roll, zoom is uniform across depth planes
(parallax scales only the pan), and every zooming frame fits SVG plates at a new size, as a
classic zoom does.

### Manually with the CLI

```bash
aether-motion check  examples/premium-promo.html                                       # validate
aether-motion render examples/premium-promo.html --output frame.png --frame 90         # preview a frame
aether-motion render examples/premium-promo.html --output promo.mp4 --size 1080p --fps 30
aether-motion render examples/variables.html --output q4.mp4 --variables-file examples/variables.json   # same template, new values
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
- **Deterministic:** same input, same frames, same output, byte for byte for any `--threads` value, render split, or CPU (vectorized paths produce the same bits as scalar ones). Generic families (`sans-serif`, `serif`, `monospace`, and the fallback) always use the embedded Roboto, italic included (synthesized), so their text matches across machines; a named family matches only where the same font file is installed.
- **Honest errors:** unsupported HTML or CSS returns the exact problem and a suggested fix, never a silently wrong video.
- **Native motion:** keyframes with expo, back, spring (`spring(overshoot, oscillations)`), and `cubic-bezier` easing, CSS `@keyframes`, camera zoom and pan with subject tracking and shot presets, kinetic text, reveals, counters, and crossfade, push, cover, wipe, and zoom transitions.
- **Captions:** `data-hf-captions` turns a transcript (`transcript.json`, OpenAI Whisper or whisper.cpp JSON, SRT, VTT) into word-highlighted captions; `aether-motion captions` converts transcripts to SRT or VTT. See `examples/captions.html`.
- **Typography:** wrapping, rich inline runs (`<span>`, `<strong>`, `<em>`, `<code>`), italics (synthesized when no italic face is installed), pair kerning with `font-kerning: normal`, outline and gradient text, and kinetic text with `data-hf-split`: combined position, scale, rotation, and opacity entrances, stagger orders, per-word or per-line masks, `<br>` lines, and automatic wrapping (`data-hf-split-wrap`) with animated lines; see `examples/kinetic-wrap.html`.
- **Video clips:** local H.264 MP4 `<video>` without B-frames (its sound included) with trim (`data-media-start`), loop, `object-fit`, and the usual fades, transforms, and clipping; see `examples/video.html`.
- **Slow motion and speed ramps:** `data-hf-speed` (constant rate) and `data-hf-speed-ramp` (eased keyframes, down to a freeze at 0) on `<video>` and `<audio>`; picture and sound stay in sync; see `examples/speed.html`.
- **Sample-accurate audio:** `<audio>` clips (WAV, FLAC, Ogg Vorbis, MP3) with volume, fades, trim, and loop are mixed and encoded to AAC with no FFmpeg; see `examples/audio.html`.
- **Audio effects:** volume automation (`data-hf-volume-ramp`), music ducking under the voice with look-ahead (`data-hf-duck`), and a parametric equalizer with presets (`data-hf-eq`), all reported by `check`; see `examples/audio-effects.html`.
- **Templates:** declare typed variables (`string`, `number`, `color`, `boolean`, `enum`, `image`, `font`) on the root with `data-composition-variables`, use them through `var(--id)`, `data-var-text`, `data-var-src`, and `data-var-if`, and render one composition many times with `--variables`, `aether.json`, or the MCP `variables` argument. Wrong ids and wrong types are errors. See `examples/variables.html`.
- **Polished look:** flexbox, text wrapping, gradients, layered and inset shadows, blur, frosted glass, rotation, CSS color filters and `drop-shadow`, `mix-blend-mode`, a root color grade, localized bloom (`data-hf-bloom`, see `examples/bloom.html`), shutter motion blur for the whole frame or one element (`data-hf-motion-blur`, see `examples/layer-motion-blur.html`), vignette, and film grain.
- **Easy on your machine:** renders at low priority on a quarter of your cores by default, with a live progress line and ETA (`--progress json` for scripts). `--threads N` goes faster; `--scale 0.5` gives quick drafts.
- **Quality presets:** `--quality draft|standard|high|max` sets the H.264 quantizer (30, 26, 22, 18); `standard` is the default, and the other presets trade file size against fidelity at about the same CPU cost.
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
