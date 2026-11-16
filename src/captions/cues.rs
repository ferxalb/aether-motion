//! Cue grouping: turns a flat list of timed words into readable cues.
//!
//! A cue ends after sentence-final punctuation, after a pause longer than `pause` seconds, or before
//! a word that would exceed `max_words` or `max_chars`. Grouping is a pure function of the words, so
//! the same transcript always yields the same cues.
use super::{Cue, Word};

/// Grouping limits.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct GroupOptions {
    pub max_words: usize,
    pub max_chars: usize,
    /// Silence (seconds) between two words that forces a new cue.
    pub pause: f64,
}

impl Default for GroupOptions {
    fn default() -> Self {
        Self {
            max_words: 8,
            max_chars: 40,
            pause: 0.8,
        }
    }
}

/// True when `text` ends a sentence (`.`, `?`, `!`, `…`, or their CJK forms), ignoring closing quotes.
fn ends_sentence(text: &str) -> bool {
    text.trim_end_matches(['"', '\'', ')', ']', '”', '’', '」', '』'])
        .ends_with(['.', '?', '!', '…', '。', '？', '！'])
}

/// Groups `words` into cues following `options`.
pub fn group(words: &[Word], options: GroupOptions) -> Vec<Cue> {
    let mut cues = Vec::new();
    let mut current: Vec<Word> = Vec::new();
    let mut chars = 0;
    for word in words {
        let width = word.text.chars().count();
        if let Some(last) = current.last() {
            let too_many = current.len() >= options.max_words;
            // +1 for the space that joins the word to the cue.
            let too_long = chars + 1 + width > options.max_chars;
            let paused = word.start - last.end > options.pause;
            if too_many || too_long || paused || ends_sentence(&last.text) {
                cues.push(Cue {
                    words: std::mem::take(&mut current),
                });
                chars = 0;
            }
        }
        chars += if current.is_empty() { width } else { 1 + width };
        current.push(word.clone());
    }
    if !current.is_empty() {
        cues.push(Cue { words: current });
    }
    cues
}

/// One cue per word (`--preserve-cues` for single-word or CJK captions).
pub fn one_per_word(words: &[Word]) -> Vec<Cue> {
    words
        .iter()
        .map(|w| Cue {
            words: vec![w.clone()],
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn w(text: &str, start: f64, end: f64) -> Word {
        Word {
            text: text.to_string(),
            start,
            end,
        }
    }

    #[test]
    fn breaks_on_sentence_end_and_pause() {
        let words = [w("Hi", 0.0, 0.2), w("there.", 0.2, 0.5), w("Next", 0.6, 0.9), w("one", 2.5, 2.8)];
        let cues = group(&words, GroupOptions::default());
        let texts: Vec<_> = cues.iter().map(Cue::text).collect();
        assert_eq!(texts, ["Hi there.", "Next", "one"]);
    }

    #[test]
    fn respects_word_and_char_limits() {
        let words: Vec<_> = (0..5).map(|i| w("abcd", i as f64, i as f64 + 0.5)).collect();
        let by_words = group(&words, GroupOptions { max_words: 2, ..Default::default() });
        assert_eq!(by_words.len(), 3);
        // "abcd abcd" is 9 chars, so a limit of 8 keeps one word per cue.
        let by_chars = group(&words, GroupOptions { max_chars: 8, ..Default::default() });
        assert_eq!(by_chars.len(), 5);
    }

    #[test]
    fn closing_quotes_do_not_hide_the_sentence_end() {
        assert!(ends_sentence("done.\""));
        assert!(ends_sentence("なに？"));
        assert!(!ends_sentence("so,"));
    }
}
