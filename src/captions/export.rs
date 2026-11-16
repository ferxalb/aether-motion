//! Caption export: writes cues as SRT, WebVTT, or the normalized `transcript.json` word array.
use super::{Cue, Word};
use serde_json::{json, Value};

/// Formats whole seconds plus milliseconds as `HH:MM:SS<sep>mmm`; times are rounded to the millisecond.
fn clock(seconds: f64, separator: char) -> String {
    let total = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}:{:02}{}{:03}",
        total / 3_600_000,
        total / 60_000 % 60,
        total / 1000 % 60,
        separator,
        total % 1000
    )
}

/// SubRip: numbered cues with comma milliseconds.
pub fn srt(cues: &[Cue]) -> String {
    let mut out = String::new();
    for (i, cue) in cues.iter().enumerate() {
        out.push_str(&format!(
            "{}\n{} --> {}\n{}\n\n",
            i + 1,
            clock(cue.start(), ','),
            clock(cue.end(), ','),
            cue.text()
        ));
    }
    out
}

/// WebVTT: a header, then cues with dot milliseconds.
pub fn vtt(cues: &[Cue]) -> String {
    let mut out = String::from("WEBVTT\n\n");
    for cue in cues {
        out.push_str(&format!(
            "{} --> {}\n{}\n\n",
            clock(cue.start(), '.'),
            clock(cue.end(), '.'),
            cue.text()
        ));
    }
    out
}

/// The `[{text,start,end}]` word array used as `transcript.json`.
pub fn words_json(words: &[Word]) -> Value {
    Value::Array(
        words
            .iter()
            .map(|w| json!({"text": w.text, "start": round3(w.start), "end": round3(w.end)}))
            .collect(),
    )
}

fn round3(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn cue() -> Cue {
        Cue {
            words: vec![
                Word {
                    text: "Hello".into(),
                    start: 3661.5,
                    end: 3662.0,
                },
                Word {
                    text: "world".into(),
                    start: 3662.0,
                    end: 3662.25,
                },
            ],
        }
    }

    #[test]
    fn writes_srt_and_vtt_timestamps() {
        assert_eq!(
            srt(&[cue()]),
            "1\n01:01:01,500 --> 01:01:02,250\nHello world\n\n"
        );
        assert_eq!(
            vtt(&[cue()]),
            "WEBVTT\n\n01:01:01.500 --> 01:01:02.250\nHello world\n\n"
        );
    }
}
