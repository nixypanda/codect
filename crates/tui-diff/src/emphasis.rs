//! Intra-line emphasis for changed diff rows.
//!
//! This is presentation-only arithmetic: it compares the old and new text of
//! one aligned row and returns the byte ranges that differ on each side.

/// The changed byte ranges of one side.
pub type Emphasis = Vec<(usize, usize)>;

/// The changed ranges of `old` and `new`, excluding the common prefix and
/// suffix.
pub fn emphasis_ranges(old: &str, new: &str) -> (Emphasis, Emphasis) {
    if old == new {
        return (Vec::new(), Vec::new());
    }

    let old_chars: Vec<(usize, char)> = old.char_indices().collect();
    let new_chars: Vec<(usize, char)> = new.char_indices().collect();

    let mut prefix = 0;
    while prefix < old_chars.len()
        && prefix < new_chars.len()
        && old_chars[prefix].1 == new_chars[prefix].1
    {
        prefix += 1;
    }

    let mut suffix = 0;
    while suffix < old_chars.len().saturating_sub(prefix)
        && suffix < new_chars.len().saturating_sub(prefix)
        && old_chars[old_chars.len() - 1 - suffix].1 == new_chars[new_chars.len() - 1 - suffix].1
    {
        suffix += 1;
    }

    let old_start = old_chars.get(prefix).map_or(old.len(), |(byte, _)| *byte);
    let old_end = old_chars
        .get(old_chars.len() - suffix)
        .map_or(old.len(), |(byte, _)| *byte);
    let new_start = new_chars.get(prefix).map_or(new.len(), |(byte, _)| *byte);
    let new_end = new_chars
        .get(new_chars.len() - suffix)
        .map_or(new.len(), |(byte, _)| *byte);

    let mut old_ranges = Vec::new();
    let mut new_ranges = Vec::new();
    if old_start < old_end {
        old_ranges.push((old_start, old_end));
    }
    if new_start < new_end {
        new_ranges.push((new_start, new_end));
    }
    (old_ranges, new_ranges)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emphasis_marks_only_the_changed_middle() {
        let (old, new) = emphasis_ranges("pub id: u32;", "pub id: u64;");
        assert_eq!(old, vec![(9, 11)]);
        assert_eq!(new, vec![(9, 11)]);

        let (old, new) = emphasis_ranges("same", "same");
        assert!(old.is_empty() && new.is_empty());
    }

    #[test]
    fn emphasis_survives_a_fully_different_line() {
        let (old, new) = emphasis_ranges("abc", "xyz");
        assert_eq!(old, vec![(0, 3)]);
        assert_eq!(new, vec![(0, 3)]);
    }
}
