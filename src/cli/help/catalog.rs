//! The command catalog: every subcommand's group, summary, usage, options, notes, and examples.
//!
//! This is the single source of the CLI's help text. Keep each entry in step with the flags its
//! command actually parses (`cli/<command>.rs` and `cli/options.rs`); a test checks that every
//! dispatched command has an entry.

/// One option row: the flag spelling (with aliases), its value placeholder, and what it does.
pub struct Opt {
    pub flags: &'static str,
    pub value: &'static str,
    pub help: &'static str,
}

/// A titled group of options.
pub struct Section {
    pub title: &'static str,
    pub options: &'static [Opt],
}

/// The help entry of one command.
pub struct Command {
    pub name: &'static str,
    /// Arguments shown after the name in the command overview, such as `[file]`.
    pub args: &'static str,
    pub group: Group,
    pub summary: &'static str,
    pub usage: &'static [&'static str],
    /// Paragraphs printed between the usage and the options.
    pub about: &'static [&'static str],
    pub sections: &'static [Section],
    /// `(command line, what it does)`.
    pub examples: &'static [(&'static str, &'static str)],
}

/// Overview groups, in display order.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Group {
    Create,
    Compose,
    Agents,
    Diagnose,
}

impl Group {
    pub const ALL: [Group; 4] = [
        Group::Create,
        Group::Compose,
        Group::Agents,
        Group::Diagnose,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Group::Create => "Create and export",
            Group::Compose => "Compose",
            Group::Agents => "Coding agents",
            Group::Diagnose => "Diagnose",
        }
    }
}

const fn opt(flags: &'static str, value: &'static str, help: &'static str) -> Opt {
    Opt { flags, value, help }
}

const VARIABLES: Section = Section {
    title: "Variables",
    options: &[
        opt(
            "--variables",
            "<json>",
            "Values for the composition's declared variables, as a JSON object keyed by id. They override \"variables\" in aether.json id by id.",
        ),
        opt(
            "--variables-file",
            "<path>",
            "The same JSON object read from a file. Use one of --variables and --variables-file.",
        ),
    ],
};

pub const COMMANDS: &[Command] = &[
    Command {
        name: "init",
        args: "[dir]",
        group: Group::Create,
        summary: "Create a project with a sample video, agent skills, and MCP config",
        usage: &["aether-motion init [dir] [options]"],
        about: &[
            "Writes aether.json, index.html, assets/, AGENTS.md, and .gitignore into <dir> (default: the current directory), then installs agent skills and MCP config. Existing files are kept unless --force is given.",
        ],
        sections: &[Section {
            title: "Options",
            options: &[
                opt(
                    "--template",
                    "promo|blank",
                    "Starting composition: promo, a 15 s sample promo (default), or blank, a 4 s title card.",
                ),
                opt("--no-agent", "", "Skip agent skills and MCP config."),
                opt(
                    "--all",
                    "",
                    "Configure MCP for every supported agent (Claude Code, Cursor, VS Code), not only the detected ones.",
                ),
                opt(
                    "-f, --force",
                    "",
                    "Overwrite existing project files and locally modified skills.",
                ),
                opt("-y, --yes", "", "Accepted for scripts; init never asks questions."),
            ],
        }],
        examples: &[
            ("aether-motion init my-video", "New project in ./my-video"),
            (
                "aether-motion init . --template blank --no-agent",
                "Blank composition here, without agent files",
            ),
        ],
    },
    Command {
        name: "preview",
        args: "[file]",
        group: Group::Create,
        summary: "Render one frame to .aether/preview.png and open it",
        usage: &[
            "aether-motion preview [options]           (project mode)",
            "aether-motion preview <file> [options]    (standalone)",
        ],
        about: &[
            "Draws one frame at the composition's own size. Without a file it previews the project's entry with the fps and variable values from aether.json; with a file it uses only the command line (30 fps unless --fps is given) and never reads aether.json. The frame rate sets the motion-blur shutter window.",
            "The PNG is replaced only once the new frame is written, so a failed render keeps the last good preview.",
        ],
        sections: &[
            Section {
                title: "Options",
                options: &[
                    opt(
                        "--at",
                        "<seconds>",
                        "Time of the frame (default 1). Times past the end show the last frame.",
                    ),
                    opt(
                        "--watch",
                        "",
                        "Render again when the composition, a scene or asset it reads, or the variables file changes. Stop with Ctrl-C.",
                    ),
                    opt(
                        "--fps",
                        "<n>",
                        "Frame rate, 1 to 240, for motion blur (default: aether.json in project mode, else 30).",
                    ),
                    opt("--no-open", "", "Write the PNG without opening a viewer."),
                ],
            },
            VARIABLES,
        ],
        examples: &[
            ("aether-motion preview --at 2.5", "Frame at 2.5 s"),
            ("aether-motion preview scene.html --watch", "Live preview while editing"),
        ],
    },
    Command {
        name: "render",
        args: "[file]",
        group: Group::Create,
        summary: "Export an MP4 or PNG",
        usage: &[
            "aether-motion render [-o <path>] [options]           (project mode)",
            "aether-motion render <file> [-o <path>] [options]    (direct mode)",
        ],
        about: &[
            "Project mode, without a file: renders the project's entry with fps, size, quality, the output folder, and variable values from aether.json. The default output is <out>/<name>.mp4, folder \"out\" from aether.json.",
            "Direct mode, with a file: only the command line counts and aether.json is never read. Defaults are 30 fps, the composition's own size, quality standard, and out/<name>.mp4 in the current directory.",
            "Both modes take every option below. A PNG is overwritten. An existing MP4 or .aseg named with -o/--output needs --force; the default output is replaced on every render.",
        ],
        sections: &[
            Section {
                title: "Output",
                options: &[
                    opt(
                        "-o, --output",
                        "<path>",
                        "Output file: .mp4, .png, or .aseg (a part of a distributed render).",
                    ),
                    opt(
                        "-f, --force",
                        "",
                        "Replace an existing MP4 or .aseg output. PNGs never need it.",
                    ),
                    opt("--frame", "<n>", "PNG only: the frame to draw (default 0)."),
                ],
            },
            Section {
                title: "Picture",
                options: &[
                    opt(
                        "--size",
                        "1080p|4k",
                        "Delivery size for 16:9 (1920x1080, 3840x2160) and 9:16 (1080x1920, 2160x3840) compositions.",
                    ),
                    opt(
                        "--scale",
                        "<factor>",
                        "Multiply the composition's size instead (default 1). Use either --size or --scale.",
                    ),
                    opt(
                        "--fps",
                        "<n>",
                        "Frame rate, 1 to 240 (default: aether.json in project mode, else 30).",
                    ),
                ],
            },
            Section {
                title: "Encoding",
                options: &[
                    opt(
                        "--quality",
                        "<preset>",
                        "draft, standard (default), high, or max: H.264 quantizer 30, 26, 22, or 18. Higher presets give larger files at about the same CPU cost.",
                    ),
                    opt(
                        "--audio-bitrate",
                        "<kbps>",
                        "AAC bitrate for compositions with sound, 64 to 320 (default 192).",
                    ),
                ],
            },
            VARIABLES,
            Section {
                title: "Performance and progress",
                options: &[
                    opt(
                        "--threads",
                        "<n>",
                        "Render threads, 1 to 64 (default: a quarter of the cores, 1 to 4). The output never depends on it.",
                    ),
                    opt(
                        "--3d-backend",
                        "cpu|gpu|cuda|auto",
                        "Backend for data-hf-3d scenes. Defaults to cpu for deterministic output. Standard builds include wgpu support; cuda (NVIDIA, Linux) needs a build with the cuda feature and the kernel from tools/build_cuda_kernel.py.",
                    ),
                    opt(
                        "--require-gpu",
                        "",
                        "With --3d-backend gpu or cuda: require an NVIDIA Vulkan adapter (gpu) or CUDA device (cuda), fail the render on any GPU error instead of falling back to the CPU, and print an AETHER_3D_PROOF line on stderr.",
                    ),
                    opt(
                        "--progress",
                        "auto|json|off",
                        "Progress on stderr: a status line on a terminal (auto, default), one JSON object per update, or nothing.",
                    ),
                ],
            },
            Section {
                title: "Stills and distributed renders",
                options: &[
                    opt(
                        "--frames",
                        "<n,n,...>",
                        "Several PNG frames from one process; the output name must contain {frame}.",
                    ),
                    opt(
                        "--encoder",
                        "native|ffmpeg",
                        "MP4 encoder (default native). ffmpeg needs FFmpeg on PATH and rejects <audio>.",
                    ),
                    opt(
                        "--priority",
                        "low|normal",
                        "Process priority (default low).",
                    ),
                    opt(
                        "--segments",
                        "<a..b>",
                        "With an .aseg output: render one-second segments a to b (end excluded). `inspect --fps N` prints the count.",
                    ),
                    opt(
                        "--parts",
                        "<a.aseg,...>",
                        "Assemble part files into the .mp4 output, byte-identical to a single render.",
                    ),
                ],
            },
        ],
        examples: &[
            ("aether-motion render", "Export the project with its aether.json settings"),
            (
                "aether-motion render promo.html -o promo.mp4 --size 4k --fps 60 --force",
                "4K60 export of one file, replacing an earlier promo.mp4",
            ),
            (
                "aether-motion render promo.html --output frame.png --frame 90",
                "Frame 90 as PNG",
            ),
            (
                "aether-motion render promo.html --output 'f-{frame}.png' --frames 0,45,90",
                "Three stills in one run",
            ),
        ],
    },
    Command {
        name: "check",
        args: "[file]",
        group: Group::Create,
        summary: "Validate a composition and probe its text against safe zones",
        usage: &[
            "aether-motion check [options]           (project mode)",
            "aether-motion check <file> [options]    (standalone)",
        ],
        about: &[
            "Without a file it checks the project's entry with the variable values from aether.json; with a file it uses only the command line and never reads aether.json.",
            "Prints a summary on a terminal and the full JSON report when stdout is piped or with --json. Text is probed at 9 midpoint times: 9:16 frames against the TikTok, Reels, and Shorts zones, other frames against 10% standard margins.",
            "Exit codes: 0 valid, 1 invalid or a gating finding, 2 usage error. Safe-zone findings are warnings and never fail.",
        ],
        sections: &[
            Section {
                title: "Output",
                options: &[opt("--json", "", "Print the JSON report on a terminal too.")],
            },
            Section {
                title: "Layout probe",
                options: &[
                    opt(
                        "--safe-zone",
                        "<zones>",
                        "tiktok, reels, shorts, standard, or off; a comma list checks several.",
                    ),
                    opt(
                        "--caption-zone",
                        "<spec>",
                        "Flag other text inside a caption band: \"x0=0;y0=0.7;x1=1;y1=1[;severity=error]\" (fractions of the frame).",
                    ),
                    opt("--samples", "<n>", "Number of probe times (default 9 midpoints)."),
                    opt("--at", "<t1,t2,...>", "Probe these times in seconds instead."),
                    opt("--strict", "", "Make caption-zone warnings fail the check."),
                ],
            },
            VARIABLES,
        ],
        examples: &[
            ("aether-motion check", "Validate the project's composition"),
            (
                "aether-motion check vertical.html --safe-zone tiktok --caption-zone \"x0=0;y0=0.7;x1=1;y1=1\"",
                "TikTok zones plus a caption band",
            ),
        ],
    },
    Command {
        name: "add",
        args: "[list|block]",
        group: Group::Compose,
        summary: "List blocks, or append one to the composition",
        usage: &["aether-motion add [list]", "aether-motion add <block> [--set key=value]..."],
        about: &[
            "With no arguments or with `list`, shows the catalog. A block name inserts its HTML before the last </div> of the project's composition. To use a theme, print it with `aether-motion block <theme>` and paste it into the composition.",
        ],
        sections: &[Section {
            title: "Options",
            options: &[opt(
                "--set",
                "<key=value>",
                "Set a block parameter (repeatable). Unset parameters keep their defaults.",
            )],
        }],
        examples: &[(
            "aether-motion add hero-title --set id=intro --set title=\"Launch day\"",
            "Append a hero title",
        )],
    },
    Command {
        name: "block",
        args: "[name]",
        group: Group::Compose,
        summary: "Print a ready-made block or theme as HTML",
        usage: &["aether-motion block [list]", "aether-motion block <name> [--set key=value]..."],
        about: &["Prints to stdout, so the HTML can be piped or pasted. Blocks target 1920x1080 and read colors from custom properties that the theme blocks set."],
        sections: &[Section {
            title: "Options",
            options: &[opt("--set", "<key=value>", "Set a parameter (repeatable).")],
        }],
        examples: &[
            ("aether-motion block list", "Every block and theme with its parameters"),
            ("aether-motion block theme-midnight", "Print a theme"),
        ],
    },
    Command {
        name: "captions",
        args: "<file>",
        group: Group::Compose,
        summary: "Convert a transcript to JSON, SRT, or VTT",
        usage: &["aether-motion captions <transcript.json|.srt|.vtt> [options]"],
        about: &[
            "Reads a word array, Whisper or whisper.cpp JSON, SRT, or WebVTT and writes it in another format. It does not transcribe audio. SRT and VTT sources keep their own cues; the grouping options apply to word sources.",
        ],
        sections: &[
            Section {
                title: "Output",
                options: &[
                    opt(
                        "--to",
                        "json|srt|vtt",
                        "Output format (default json: the word array, which carries no cues).",
                    ),
                    opt("-o, --output", "<path>", "Write to a file instead of stdout."),
                ],
            },
            Section {
                title: "Cue grouping",
                options: &[
                    opt("--max-words", "<n>", "Words per cue (default 8)."),
                    opt("--max-chars", "<n>", "Characters per cue (default 40)."),
                    opt(
                        "--pause",
                        "<seconds>",
                        "Silence that starts a new cue, 0 to 10 (default 0.8).",
                    ),
                    opt("--preserve-cues", "", "One cue per entry (single words, CJK)."),
                ],
            },
        ],
        examples: &[(
            "aether-motion captions whisper.json --to srt -o captions.srt",
            "Whisper JSON to SRT",
        )],
    },
    Command {
        name: "agent",
        args: "setup|status",
        group: Group::Agents,
        summary: "Install skills and MCP config for coding agents",
        usage: &["aether-motion agent [setup] [options]", "aether-motion agent status"],
        about: &[
            "setup (the default) writes the bundled skills to .claude/skills and .agents/skills and registers the MCP server in the project config of each detected agent. status reports what is installed, like doctor. Nothing outside the project is touched.",
        ],
        sections: &[Section {
            title: "Options",
            options: &[
                opt(
                    "--all",
                    "",
                    "Configure every supported agent (Claude Code, Cursor, VS Code), not only the detected ones.",
                ),
                opt("-f, --force", "", "Overwrite locally modified skill files."),
            ],
        }],
        examples: &[("aether-motion agent setup --all", "Wire up every supported agent")],
    },
    Command {
        name: "mcp",
        args: "",
        group: Group::Agents,
        summary: "Run the MCP server over stdio",
        usage: &["aether-motion mcp [--selftest | --stdio]"],
        about: &[
            "Speaks the Model Context Protocol on stdin and stdout for coding agents; the agent starts it, so a run by hand in a terminal prints setup help instead of waiting. Tools in workflow order: aether_guide (learn), aether_block and aether_write (author), aether_check and aether_snapshot (verify), aether_render (deliver).",
        ],
        sections: &[Section {
            title: "Options",
            options: &[
                opt("--selftest", "", "Run the handshake and tool list in-process and print them, without a client."),
                opt("--stdio", "", "Serve even when stdin is a terminal."),
            ],
        }],
        examples: &[
            (
                "claude mcp add aether-motion -- aether-motion mcp",
                "Register the server with Claude Code",
            ),
            ("aether-motion mcp --selftest", "Check that the server answers"),
        ],
    },
    Command {
        name: "doctor",
        args: "",
        group: Group::Diagnose,
        summary: "Check the installation, the project, and agent setup",
        usage: &["aether-motion doctor"],
        about: &["Validates the project's composition, looks for FFmpeg (optional), and checks that skills and MCP config are installed, with a fix for each problem."],
        sections: &[],
        examples: &[],
    },
    Command {
        name: "inspect",
        args: "<file>",
        group: Group::Diagnose,
        summary: "Print the canvas, timed elements, audio, and segment plan",
        usage: &["aether-motion inspect <file> [options]"],
        about: &["Lists the resolved size, duration, element count, audio clips, and every element that is visible for only part of the composition. aether.json is not read."],
        sections: &[
            Section {
                title: "Options",
                options: &[opt(
                    "--fps",
                    "<n>",
                    "Also print the frame count and the number of one-second segments for distributed renders.",
                )],
            },
            VARIABLES,
        ],
        examples: &[("aether-motion inspect promo.html --fps 30", "Timeline and segment plan")],
    },
    Command {
        name: "bench",
        args: "<file>",
        group: Group::Diagnose,
        summary: "Time frame drawing without encoding",
        usage: &["aether-motion bench <file> [options]"],
        about: &["Draws frames on one renderer and reports the time per frame. aether.json is not read. Set AETHER_PROFILE=1 to split the time by phase."],
        sections: &[
            Section {
                title: "Options",
                options: &[
                    opt("--frame", "<start>", "First frame (default 0)."),
                    opt("--count", "<n>", "Frames to draw (default: to the end)."),
                    opt("--fps", "<n>", "Frame rate, 1 to 240 (default 30)."),
                    opt("--size", "1080p|4k", "Delivery size, as in render."),
                    opt("--scale", "<factor>", "Size factor instead of --size (default 1)."),
                    opt("--priority", "low|normal", "Process priority (default low)."),
                    opt("--3d-backend", "cpu|gpu|cuda|auto", "Backend for data-hf-3d scenes."),
                ],
            },
            VARIABLES,
        ],
        examples: &[(
            "aether-motion bench promo.html --frame 90 --count 20",
            "Time 20 frames from frame 90",
        )],
    },
];

/// The entry for `name`.
pub fn find(name: &str) -> Option<&'static Command> {
    COMMANDS.iter().find(|c| c.name == name)
}
