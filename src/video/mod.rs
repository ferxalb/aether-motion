//! `<video>` support: H.264 MP4 files decoded natively, one source frame per output frame.
//!
//! `index` reads only the container's sample tables (no decoding) and turns them into a display-order
//! frame table plus closed groups of pictures (GOPs); it runs at parse time, so a composition knows
//! each video's length and frame times. `player` decodes on demand: it seeks to the keyframe that
//! starts the GOP holding the wanted frame and decodes forward, so frames can be requested in any
//! order (each render thread owns its own player). `colour` reads the SPS colour tags and converts
//! pictures that need more than the decoder's built-in conversion. `annexb` converts MP4 length-prefixed NAL units
//! to the Annex B form the decoder reads.
//!
//! Scope: H.264 in a non-fragmented MP4 or MOV, closed GOPs (B-frames allowed), no rotation
//! metadata. The video's audio track is not mixed; the element must be `muted` and sound comes from
//! `<audio>`.
mod annexb;
pub mod colour;
mod index;
mod player;

pub use index::{Gop, VideoIndex, MAX_FRAMES, MAX_PIXELS};
pub use player::VideoPlayer;
