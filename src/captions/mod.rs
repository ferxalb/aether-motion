//! Captions domain: timed words and cues, independent of HTML parsing and rendering.
//!
//! `transcript` imports `transcript.json`, OpenAI Whisper and whisper.cpp JSON, SRT, and VTT;
//! `cues` groups words into readable cues; `export` writes SRT, VTT, and the normalized word array.
//! `parse/captions.rs` turns cues into composition elements (`data-hf-captions`) and the
//! `captions` CLI command uses this module for conversion. Audio is never transcribed here.

pub mod cues;
pub mod export;
pub mod transcript;

/// One spoken word with times in seconds.
#[derive(Clone, Debug, PartialEq)]
pub struct Word {
    pub text: String,
    pub start: f64,
    pub end: f64,
}

/// Words shown together.
#[derive(Clone, Debug, PartialEq)]
pub struct Cue {
    pub words: Vec<Word>,
}

impl Cue {
    pub fn start(&self) -> f64 {
        self.words.first().map_or(0.0, |w| w.start)
    }

    pub fn end(&self) -> f64 {
        self.words.last().map_or(0.0, |w| w.end)
    }

    pub fn text(&self) -> String {
        self.words
            .iter()
            .map(|w| w.text.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
