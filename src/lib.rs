//! Library facade: `model` (data), `parse` (input), `render` (frames), `encode` (MP4), `agent` (check and MCP), `audio` (decode, mix, and AAC for `<audio>`), `blocks` (ready-made blocks and themes), `captions` (transcript import, cue grouping, SRT/VTT export), `cli` (human-facing subcommands), `setup` (agent skills and MCP config).

pub mod agent;
pub mod audio;
pub mod blocks;
pub mod captions;
pub mod cli;
pub mod encode;
pub mod model;
pub mod parse;
pub mod render;
pub mod setup;
pub mod video;
