//! Line-oriented patience diff over canonical projected text.
//!
//! The diff engine compares projection text rather than syntax trees; output is
//! plain and deterministic, with ANSI styling applied only in the CLI.

use similar::{Algorithm, TextDiff};

/// Context lines emitted around each change. Three is the conventional default
/// and keeps focused diffs readable without depending on terminal width
/// (TECHNICAL_DESIGN.md section 13).
const CONTEXT_RADIUS: usize = 3;

/// The unified hunks comparing two canonical projection texts.
///
/// Returns an empty string when the texts are equal, so a caller can treat
/// "empty" as "no focused change" (TECHNICAL_DESIGN.md section 7.2). Hunks use
/// the standard `@@ -a,b +c,d @@` header and `-`, `+`, and space prefixes. File
/// headers and the trailing newline of the enclosing document belong to
/// [`crate::render`].
///
/// An empty string is treated as zero lines, matching the core invariant that a
/// projection with no items has empty canonical text.
pub fn unified_hunks(old: &str, new: &str) -> String {
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .diff_lines(old, new);

    let mut unified = diff.unified_diff();
    unified.context_radius(CONTEXT_RADIUS);
    unified.to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn equal_texts_produce_no_hunks() {
        assert!(unified_hunks("", "").is_empty());
        assert!(unified_hunks("a\nb\n", "a\nb\n").is_empty());
    }

    #[test]
    fn pure_insertion_produces_one_hunk() {
        let hunks = unified_hunks("a\nc\n", "a\nb\nc\n");
        assert_eq!(hunks, "@@ -1,2 +1,3 @@\n a\n+b\n c\n");
    }

    #[test]
    fn pure_deletion_produces_one_hunk() {
        let hunks = unified_hunks("a\nb\nc\n", "a\nc\n");
        assert_eq!(hunks, "@@ -1,3 +1,2 @@\n a\n-b\n c\n");
    }

    #[test]
    fn modification_marks_the_old_and_new_line() {
        let hunks = unified_hunks("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(hunks, "@@ -1,3 +1,3 @@\n a\n-b\n+B\n c\n");
    }

    #[test]
    fn empty_versus_nonempty_uses_an_empty_range() {
        assert_eq!(
            unified_hunks("", "a\n"),
            "@@ -0,0 +1 @@\n+a\n",
            "an absent side must diff against zero lines"
        );
        assert_eq!(
            unified_hunks("a\n", ""),
            "@@ -1 +0,0 @@\n-a\n",
            "a removed side must diff against zero lines"
        );
    }

    #[test]
    fn distant_changes_split_into_hunks_with_three_context_lines() {
        // Changes on line 2 and line 12 leave more than six unchanged lines
        // between them, so patience must split the result into two hunks and
        // each hunk must keep exactly three context lines (section 13).
        let old: String = (1..=20).map(|n| format!("line {n}\n")).collect();
        let mut new_lines: Vec<String> = (1..=20).map(|n| format!("line {n}\n")).collect();
        new_lines[1] = "CHANGED 2\n".to_owned();
        new_lines[11] = "CHANGED 12\n".to_owned();
        let new: String = new_lines.concat();

        let hunks = unified_hunks(&old, &new);
        let headers: Vec<&str> = hunks
            .lines()
            .filter(|line| line.starts_with("@@"))
            .collect();
        assert_eq!(headers, vec!["@@ -1,5 +1,5 @@", "@@ -9,7 +9,7 @@"]);

        // Exactly three context lines surround each changed line.
        assert!(hunks.contains(" line 1\n-line 2\n+CHANGED 2\n line 3\n line 4\n line 5\n"));
        assert!(
            hunks.contains(
                " line 9\n line 10\n line 11\n-line 12\n+CHANGED 12\n line 13\n line 14\n line 15\n"
            ),
            "each hunk must keep three context lines: {hunks:?}"
        );
    }

    #[test]
    fn output_never_contains_escape_bytes() {
        let hunks = unified_hunks("a\n", "b\n");
        assert!(!hunks.contains('\u{1b}'), "core output must be plain text");
    }
}
