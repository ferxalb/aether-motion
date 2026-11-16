//! Library facade: `model` (data), `parse` (input), `render` (frames), `encode` (MP4), `agent` (check and MCP), `audio` (decode, mix, and AAC for `<audio>`), `blocks` (ready-made blocks and themes), `cli` (human-facing subcommands), `setup` (agent skills and MCP config).

pub mod agent;
pub mod audio;
pub mod blocks;
pub mod cli;
pub mod encode;
pub mod model;
pub mod parse;
pub mod render;
pub mod setup;
