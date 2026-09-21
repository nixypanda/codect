//! A small case-insensitive subsequence matcher for the command palette and the
//! file finder. It returns the matched byte offsets so the view can highlight
//! them. Pure and allocation-light; no external dependency.

/// A successful match: a higher score is a better match, and `positions` are the
/// byte offsets of the matched characters in the haystack.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Match {
    pub score: i32,
    pub positions: Vec<usize>,
}

/// Matches `needle` as a subsequence of `haystack`, case-insensitively.
///
/// Consecutive matches and matches at word boundaries score higher, so
/// `scp` prefers `src/core/paths.rs` over an arbitrary scattered match.
pub fn fuzzy(needle: &str, haystack: &str) -> Option<Match> {
    let needle: Vec<char> = needle.chars().flat_map(char::to_lowercase).collect();
    if needle.is_empty() {
        return Some(Match {
            score: 0,
            positions: Vec::new(),
        });
    }

    let hay: Vec<(usize, char)> = haystack.char_indices().collect();
    let mut positions = Vec::with_capacity(needle.len());
    let mut score = 0i32;
    let mut matched = 0usize;
    let mut previous: Option<usize> = None;

    for (index, &(byte, character)) in hay.iter().enumerate() {
        if matched == needle.len() {
            break;
        }
        let lower = character.to_lowercase().next().unwrap_or(character);
        if lower != needle[matched] {
            continue;
        }
        positions.push(byte);
        score += 10;
        if index.checked_sub(1) == previous {
            score += 25;
        }
        let at_boundary =
            index == 0 || !hay[index - 1].1.is_alphanumeric() || hay[index - 1].1.is_uppercase();
        if at_boundary {
            score += 10;
        }
        // A shorter haystack is a tighter match.
        score -= (hay.len() / 16) as i32;
        previous = Some(index);
        matched += 1;
    }

    if matched == needle.len() {
        Some(Match { score, positions })
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_empty_needle_matches_everything() {
        let m = fuzzy("", "anything").expect("empty matches");
        assert_eq!(m.score, 0);
        assert!(m.positions.is_empty());
    }

    #[test]
    fn a_subsequence_matches_and_reports_offsets() {
        let m = fuzzy("scp", "src/core/paths.rs").expect("subsequence");
        let chars: Vec<char> = "src/core/paths.rs".chars().collect();
        let matched: String = m
            .positions
            .iter()
            .map(|&byte| {
                let index = "src/core/paths.rs"[..byte].chars().count();
                chars[index]
            })
            .collect();
        assert_eq!(matched.to_lowercase(), "scp");
    }

    #[test]
    fn a_missing_character_fails() {
        assert!(fuzzy("xyz", "src/core/paths.rs").is_none());
    }

    #[test]
    fn consecutive_matches_score_higher() {
        let tight = fuzzy("path", "path.rs").unwrap().score;
        let loose = fuzzy("path", "p/a/t/h.rs").unwrap().score;
        assert!(tight > loose, "tight={tight} loose={loose}");
    }

    #[test]
    fn matching_is_case_insensitive() {
        assert!(fuzzy("SRC", "src/lib.rs").is_some());
    }
}
