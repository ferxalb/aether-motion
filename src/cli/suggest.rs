//! "Did you mean" suggestions for mistyped commands and options.

/// Edit distance between `a` and `b` (insertions, deletions, substitutions).
fn distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, cb) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = if ca == *cb {
                previous
            } else {
                1 + previous.min(row[j]).min(current)
            };
            previous = current;
        }
    }
    row[b.len()]
}

/// The candidate closest to `word`, if it is close enough to be a likely typo: at most a third of
/// the word's length away (at least 1, at most 3 edits), or a candidate that starts with `word`.
pub fn closest<'a>(word: &str, candidates: impl IntoIterator<Item = &'a str>) -> Option<&'a str> {
    let limit = (word.chars().count() / 3).clamp(1, 3);
    candidates
        .into_iter()
        .map(|c| (distance(word, c), c))
        .filter(|&(d, c)| d <= limit || (word.len() >= 3 && c.starts_with(word)))
        .min_by_key(|&(d, _)| d)
        .map(|(_, c)| c)
}

#[cfg(test)]
mod tests {
    use super::closest;

    #[test]
    fn suggests_near_misses_only() {
        let commands = ["init", "render", "preview", "check"];
        assert_eq!(closest("rendr", commands), Some("render"));
        assert_eq!(closest("chek", commands), Some("check"));
        assert_eq!(closest("prev", commands), Some("preview"));
        assert_eq!(closest("deploy", commands), None);
        assert_eq!(
            closest("--qualty", ["--quality", "--fps"]),
            Some("--quality")
        );
    }
}
