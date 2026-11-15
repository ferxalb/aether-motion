//! Greedy line breaking at spaces. Pure: it knows nothing about fonts, scale, or rendering.

/// One output line as a byte range of the source text plus its advance width.
#[derive(Clone, Debug, PartialEq)]
pub(in crate::render) struct Line {
    pub(in crate::render) start: usize,
    pub(in crate::render) end: usize,
    pub(in crate::render) width: f32,
}

/// Breaks `text` (words separated by single spaces) into lines no wider than `limit`.
///
/// `advance` gives the width of one character. A word wider than `limit` gets its own line and
/// overflows, like `overflow-wrap: normal`. `limit: None` keeps everything on one line. Empty text
/// yields one empty line so every text element has a height.
pub(in crate::render) fn break_lines(
    text: &str,
    limit: Option<f32>,
    mut advance: impl FnMut(char) -> f32,
) -> Vec<Line> {
    let space = advance(' ');
    let mut lines = Vec::new();
    let mut current: Option<Line> = None;
    let mut offset = 0;
    for word in text.split(' ') {
        let start = offset;
        offset += word.len() + 1;
        if word.is_empty() {
            continue;
        }
        let end = start + word.len();
        let width: f32 = word.chars().map(&mut advance).sum();
        current = Some(match current.take() {
            None => Line { start, end, width },
            Some(line) => {
                let joined = line.width + space + width;
                if limit.is_none_or(|limit| joined <= limit) {
                    Line {
                        start: line.start,
                        end,
                        width: joined,
                    }
                } else {
                    lines.push(line);
                    Line { start, end, width }
                }
            }
        });
    }
    lines.extend(current);
    if lines.is_empty() {
        lines.push(Line {
            start: 0,
            end: 0,
            width: 0.0,
        });
    }
    lines
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Monospace metrics: every character is 1 unit wide.
    fn lines(text: &str, limit: Option<f32>) -> Vec<&str> {
        break_lines(text, limit, |_| 1.0)
            .iter()
            .map(|line| &text[line.start..line.end])
            .collect()
    }

    #[test]
    fn keeps_one_line_without_a_limit() {
        assert_eq!(lines("aaa bbb ccc", None), ["aaa bbb ccc"]);
    }

    #[test]
    fn breaks_greedily_at_spaces() {
        assert_eq!(lines("aaa bbb ccc", Some(7.0)), ["aaa bbb", "ccc"]);
        assert_eq!(lines("aaa bbb ccc", Some(6.0)), ["aaa", "bbb", "ccc"]);
        assert_eq!(lines("aa bb cc dd", Some(5.0)), ["aa bb", "cc dd"]);
    }

    #[test]
    fn an_exact_fit_stays_on_the_line() {
        assert_eq!(lines("aaa bbb", Some(7.0)), ["aaa bbb"]);
        assert_eq!(lines("aaa bbb", Some(6.99)), ["aaa", "bbb"]);
    }

    #[test]
    fn an_overlong_word_overflows_on_its_own_line() {
        assert_eq!(lines("a abcdefgh b", Some(4.0)), ["a", "abcdefgh", "b"]);
    }

    #[test]
    fn reports_line_widths_and_empty_text() {
        let found = break_lines("aaa bb c", Some(6.0), |_| 1.0);
        assert_eq!(
            found.iter().map(|line| line.width).collect::<Vec<_>>(),
            [6.0, 1.0]
        );
        assert_eq!(break_lines("", Some(3.0), |_| 1.0).len(), 1);
    }
}
