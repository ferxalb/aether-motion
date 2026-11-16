//! `captions`: converts an existing transcript (JSON, SRT, or VTT) into `transcript.json`, SRT, or
//! VTT, grouping words into cues. It does not transcribe audio; produce a transcript with a speech
//! recognizer such as whisper.cpp and import it here.

use super::args::Args;
use crate::captions::cues::{group, one_per_word, GroupOptions};
use crate::captions::{export, transcript};
use anyhow::{bail, Result};
use std::fs;
use std::path::Path;

pub fn run(raw: &[String]) -> Result<()> {
    let args = Args::parse(
        raw,
        &["--to", "--output", "--max-words", "--max-chars", "--pause"],
        &["--preserve-cues"],
    )?;
    let [input] = args.positional.as_slice() else {
        bail!("usage: aether-motion captions <transcript.json|.srt|.vtt> [--to json|srt|vtt] [--output file] [--preserve-cues] [--max-words N] [--max-chars N] [--pause SECONDS]");
    };
    let loaded = transcript::load(Path::new(input))?;
    let defaults = GroupOptions::default();
    let options = GroupOptions {
        max_words: args.number("--max-words")?.unwrap_or(defaults.max_words),
        max_chars: args.number("--max-chars")?.unwrap_or(defaults.max_chars),
        pause: args.number("--pause")?.unwrap_or(defaults.pause),
    };
    if options.max_words == 0 || options.max_chars == 0 || !(0.0..=10.0).contains(&options.pause) {
        bail!("--max-words and --max-chars must be at least 1 and --pause between 0 and 10");
    }
    // Subtitle sources keep their own cue boundaries, as in the composition attribute.
    let cues = match (&loaded.cues, args.has("--preserve-cues")) {
        (_, true) => one_per_word(&loaded.words),
        (Some(cues), false) => cues.clone(),
        (None, false) => group(&loaded.words, options),
    };
    let text = match args.value("--to").unwrap_or("json") {
        "json" => format!("{}\n", serde_json::to_string_pretty(&export::words_json(&loaded.words))?),
        "srt" => export::srt(&cues),
        "vtt" => export::vtt(&cues),
        other => bail!("--to must be json, srt, or vtt: {other}"),
    };
    match args.value("--output") {
        Some(path) => {
            fs::write(path, text)?;
            println!("Wrote {path}");
        }
        None => print!("{text}"),
    }
    Ok(())
}
