//! Transcript import: normalizes the supported caption sources into timed words and, for SRT/VTT,
//! the original cue boundaries.
//!
//! Supported inputs (detected by extension and JSON shape):
//! - `transcript.json`: an array of `{"text", "start", "end"}` objects in seconds (`word` is accepted
//!   as an alias of `text`);
//! - OpenAI Whisper API JSON: `words` (`word`, `start`, `end`) or, without words, `segments`;
//! - whisper.cpp JSON (`-oj`, or `-ojf` for token-level words): `transcription` with millisecond
//!   `offsets`;
//! - SRT and WebVTT.
//!
//! Segment-level sources have no word times, so each segment is split into words whose times are
//! proportional to their length. Nothing here transcribes audio; it only reads existing transcripts.
use super::{Cue, Word};
use anyhow::{anyhow, bail, Context, Result};
use serde_json::Value;
use std::path::Path;

/// Largest accepted transcript file (bytes).
const MAX_BYTES: u64 = 32 * 1024 * 1024;

/// A normalized transcript. `cues` is `Some` when the source defined its own cue boundaries.
#[derive(Clone, Debug, PartialEq)]
pub struct Transcript {
    pub words: Vec<Word>,
    pub cues: Option<Vec<Cue>>,
}

/// Reads and normalizes the transcript at `path`.
pub fn load(path: &Path) -> Result<Transcript> {
    let meta = std::fs::metadata(path).with_context(|| format!("could not read {}", path.display()))?;
    if meta.len() > MAX_BYTES {
        bail!("{} is larger than {} MiB", path.display(), MAX_BYTES >> 20);
    }
    let raw = std::fs::read_to_string(path)
        .with_context(|| format!("could not read {}", path.display()))?;
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    let transcript = match ext.as_str() {
        "srt" | "vtt" => from_cues(parse_subtitles(&raw)?),
        "json" => from_json(&serde_json::from_str(&raw).context("invalid JSON")?)?,
        other => bail!("unsupported transcript format .{other}; use .json, .srt, or .vtt"),
    };
    validate(&transcript.words)?;
    if transcript.words.is_empty() {
        bail!("{} contains no words", path.display());
    }
    Ok(transcript)
}

/// Words must be finite, ordered by start, and each end must not precede its start.
fn validate(words: &[Word]) -> Result<()> {
    let mut previous = 0.0_f64;
    for (i, w) in words.iter().enumerate() {
        if !w.start.is_finite() || !w.end.is_finite() || w.start < 0.0 {
            bail!("word {} (\"{}\") has an invalid time", i + 1, w.text);
        }
        if w.end < w.start {
            bail!("word {} (\"{}\") ends before it starts", i + 1, w.text);
        }
        if w.start + 1e-9 < previous {
            bail!(
                "word {} (\"{}\") starts before the previous word; sort the transcript by time",
                i + 1,
                w.text
            );
        }
        previous = w.start;
    }
    Ok(())
}

fn from_cues(cues: Vec<Cue>) -> Transcript {
    let words = cues.iter().flat_map(|c| c.words.iter().cloned()).collect();
    Transcript {
        words,
        cues: Some(cues),
    }
}

fn from_json(value: &Value) -> Result<Transcript> {
    if let Some(items) = value.as_array() {
        return Ok(Transcript {
            words: simple_words(items)?,
            cues: None,
        });
    }
    if let Some(items) = value.get("words").and_then(Value::as_array) {
        return Ok(Transcript {
            words: simple_words(items)?,
            cues: None,
        });
    }
    if let Some(items) = value.get("segments").and_then(Value::as_array) {
        let cues = items
            .iter()
            .map(|s| {
                let (start, end) = (seconds(s, "start")?, seconds(s, "end")?);
                let text = s.get("text").and_then(Value::as_str).unwrap_or("");
                Ok(spread(text, start, end))
            })
            .collect::<Result<Vec<_>>>()?;
        return Ok(from_cues(cues.into_iter().filter(|c| !c.words.is_empty()).collect()));
    }
    if let Some(items) = value.get("transcription").and_then(Value::as_array) {
        return whisper_cpp(items);
    }
    Err(anyhow!(
        "unrecognized JSON transcript: expected an array of {{text,start,end}}, OpenAI \"words\"/\"segments\", or whisper.cpp \"transcription\""
    ))
}

fn seconds(item: &Value, key: &str) -> Result<f64> {
    item.get(key)
        .and_then(Value::as_f64)
        .ok_or_else(|| anyhow!("missing numeric \"{key}\" in {item}"))
}

/// `{text|word, start, end}` in seconds; empty words are skipped.
fn simple_word(item: &Value) -> Result<Word> {
    let text = item
        .get("text")
        .or_else(|| item.get("word"))
        .and_then(Value::as_str)
        .ok_or_else(|| anyhow!("missing \"text\" in {item}"))?;
    Ok(Word {
        text: text.trim().to_string(),
        start: seconds(item, "start")?,
        end: seconds(item, "end")?,
    })
}

/// Reads a `[{text,start,end}]` style list, dropping entries whose text is empty.
fn simple_words(items: &[Value]) -> Result<Vec<Word>> {
    let mut words = Vec::with_capacity(items.len());
    for item in items {
        let word = simple_word(item)?;
        if !word.text.is_empty() {
            words.push(word);
        }
    }
    Ok(words)
}

fn millis(item: &Value, key: &str) -> Result<f64> {
    let offsets = item
        .get("offsets")
        .ok_or_else(|| anyhow!("missing \"offsets\" in {item}"))?;
    Ok(seconds(offsets, key)? / 1000.0)
}

/// whisper.cpp: entries are segments (one cue each) unless `tokens` are present, in which case
/// words are rebuilt from the tokens (a leading space starts a new word; `[_...]` markers are skipped).
fn whisper_cpp(items: &[Value]) -> Result<Transcript> {
    let mut cues = Vec::new();
    for item in items {
        let tokens = item.get("tokens").and_then(Value::as_array);
        let cue = match tokens {
            Some(tokens) if !tokens.is_empty() => {
                let mut words: Vec<Word> = Vec::new();
                for token in tokens {
                    let text = token.get("text").and_then(Value::as_str).unwrap_or("");
                    if text.trim().is_empty() || text.trim_start().starts_with("[_") {
                        continue;
                    }
                    let (from, to) = (millis(token, "from")?, millis(token, "to")?);
                    match words.last_mut() {
                        Some(last) if !text.starts_with(' ') => {
                            last.text.push_str(text);
                            last.end = to.max(last.start);
                        }
                        _ => words.push(Word {
                            text: text.trim().to_string(),
                            start: from,
                            end: to.max(from),
                        }),
                    }
                }
                Cue { words }
            }
            _ => {
                let text = item.get("text").and_then(Value::as_str).unwrap_or("");
                spread(text, millis(item, "from")?, millis(item, "to")?)
            }
        };
        if !cue.words.is_empty() {
            cues.push(cue);
        }
    }
    // Token-level files describe words; segment-level files describe cues.
    let token_level = items
        .iter()
        .any(|i| i.get("tokens").and_then(Value::as_array).is_some_and(|t| !t.is_empty()));
    let words: Vec<Word> = cues.iter().flat_map(|c| c.words.iter().cloned()).collect();
    Ok(Transcript {
        words,
        cues: if token_level { None } else { Some(cues) },
    })
}

/// Splits `text` into words whose times divide `[start, end]` in proportion to their length.
pub(super) fn spread(text: &str, start: f64, end: f64) -> Cue {
    let parts: Vec<&str> = text.split_whitespace().collect();
    let total: usize = parts.iter().map(|p| p.chars().count()).sum();
    let span = (end - start).max(0.0);
    let mut at = start;
    let mut words = Vec::with_capacity(parts.len());
    for part in parts {
        let share = if total == 0 {
            0.0
        } else {
            span * part.chars().count() as f64 / total as f64
        };
        words.push(Word {
            text: part.to_string(),
            start: at,
            end: at + share,
        });
        at += share;
    }
    Cue { words }
}

/// Parses `HH:MM:SS,mmm`, `HH:MM:SS.mmm`, or `MM:SS.mmm` into seconds.
fn timestamp(raw: &str) -> Result<f64> {
    let normalized = raw.trim().replace(',', ".");
    let parts: Vec<&str> = normalized.split(':').collect();
    let bad = || anyhow!("invalid timestamp: {raw}");
    let (h, m, s) = match parts.as_slice() {
        [h, m, s] => (h.parse::<f64>().map_err(|_| bad())?, m.parse::<f64>().map_err(|_| bad())?, s),
        [m, s] => (0.0, m.parse::<f64>().map_err(|_| bad())?, s),
        _ => return Err(bad()),
    };
    let s: f64 = s.parse().map_err(|_| bad())?;
    Ok(h * 3600.0 + m * 60.0 + s)
}

/// SRT and WebVTT share one shape for our purposes: blocks with a `start --> end` line followed by text.
fn parse_subtitles(raw: &str) -> Result<Vec<Cue>> {
    let mut cues = Vec::new();
    let mut lines = raw.trim_start_matches('\u{feff}').lines().peekable();
    while let Some(line) = lines.next() {
        let Some((from, rest)) = line.split_once("-->") else {
            continue;
        };
        // WebVTT may append cue settings after the end time.
        let to = rest.split_whitespace().next().unwrap_or("");
        let (start, end) = (timestamp(from)?, timestamp(to)?);
        let mut text = Vec::new();
        while let Some(next) = lines.peek() {
            if next.trim().is_empty() {
                break;
            }
            text.push(strip_tags(lines.next().unwrap()));
        }
        let cue = spread(&text.join(" "), start, end);
        if !cue.words.is_empty() {
            cues.push(cue);
        }
    }
    Ok(cues)
}

/// Removes `<i>`, `<b>`, `<c.color>` style tags that subtitle files may carry.
fn strip_tags(line: &str) -> String {
    let mut out = String::with_capacity(line.len());
    let mut inside = false;
    for ch in line.chars() {
        match ch {
            '<' => inside = true,
            '>' if inside => inside = false,
            _ if !inside => out.push(ch),
            _ => {}
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn reads_plain_and_openai_words() {
        let t = from_json(&json!([{"text":" Hi ","start":0.0,"end":0.4}])).unwrap();
        assert_eq!(t.words[0].text, "Hi");
        let t = from_json(&json!({"words":[{"word":"a","start":1.0,"end":1.5}]})).unwrap();
        assert_eq!((t.words[0].start, t.words[0].end), (1.0, 1.5));
    }

    #[test]
    fn spreads_segments_by_length() {
        let t = from_json(&json!({"segments":[{"start":0.0,"end":4.0,"text":"aa bbbbbb"}]})).unwrap();
        let w = &t.words;
        assert!((w[0].end - 1.0).abs() < 1e-9 && (w[1].start - 1.0).abs() < 1e-9);
        assert!((w[1].end - 4.0).abs() < 1e-9);
        assert_eq!(t.cues.unwrap().len(), 1);
    }

    #[test]
    fn rebuilds_whisper_cpp_words_from_tokens() {
        let t = from_json(&json!({"transcription":[{
            "offsets":{"from":0,"to":900},"text":" Hello world",
            "tokens":[
                {"text":"[_BEG_]","offsets":{"from":0,"to":0}},
                {"text":" Hel","offsets":{"from":0,"to":300}},
                {"text":"lo","offsets":{"from":300,"to":500}},
                {"text":" world","offsets":{"from":500,"to":900}}]}]}))
        .unwrap();
        let texts: Vec<_> = t.words.iter().map(|w| w.text.as_str()).collect();
        assert_eq!(texts, ["Hello", "world"]);
        assert_eq!((t.words[0].start, t.words[0].end), (0.0, 0.5));
        assert!(t.cues.is_none());
    }

    #[test]
    fn parses_srt_and_vtt() {
        let srt = "1\n00:00:01,000 --> 00:00:03,000\n<i>Hello</i> there\n\n2\n00:00:03,500 --> 00:00:04,000\nBye\n";
        let cues = parse_subtitles(srt).unwrap();
        assert_eq!(cues.len(), 2);
        assert_eq!(cues[0].words[0].text, "Hello");
        assert!((cues[0].words[1].end - 3.0).abs() < 1e-9);
        let vtt = "WEBVTT\n\n00:01.000 --> 00:02.000 align:start\nHi\n";
        assert_eq!(parse_subtitles(vtt).unwrap()[0].words[0].start, 1.0);
    }

    #[test]
    fn rejects_unordered_words() {
        let words = vec![
            Word { text: "a".into(), start: 2.0, end: 3.0 },
            Word { text: "b".into(), start: 1.0, end: 1.5 },
        ];
        assert!(validate(&words).unwrap_err().to_string().contains("sort"));
    }
}
