//! Line-oriented patience diff over canonical projected text.
//!
//! The diff engine compares projection text rather than syntax trees; output is
//! plain and deterministic, with ANSI styling applied only in the CLI.
//!
//! [`aligned_rows`] and [`unified_hunks`] share one patience configuration and
//! line tokenization, so they always agree on changed lines, and equal texts
//! produce no output.

use similar::{Algorithm, DiffOp, DiffableStr, TextDiff};

use crate::model::{ProjectedFile, RepoPath};

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FileDiff {
    Added {
        new: ProjectedFile,
    },
    Deleted {
        old: ProjectedFile,
    },
    Modified {
        old: ProjectedFile,
        new: ProjectedFile,
    },
}

impl FileDiff {
    pub fn path(&self) -> &RepoPath {
        match self {
            Self::Added { new } => new.path(),
            Self::Deleted { old } => old.path(),
            Self::Modified { old, .. } => old.path(),
        }
    }
}

const CONTEXT_RADIUS: usize = 3;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiffRowKind {
    Equal,
    Add,
    Delete,
    Change,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct DiffLine<'a> {
    pub number: usize,
    pub text: &'a str,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AlignedRow<'a> {
    Equal {
        old: DiffLine<'a>,
        new: DiffLine<'a>,
    },
    Add {
        new: DiffLine<'a>,
    },
    Delete {
        old: DiffLine<'a>,
    },
    Change {
        old: DiffLine<'a>,
        new: DiffLine<'a>,
    },
}

impl AlignedRow<'_> {
    pub fn kind(&self) -> DiffRowKind {
        match self {
            Self::Equal { .. } => DiffRowKind::Equal,
            Self::Add { .. } => DiffRowKind::Add,
            Self::Delete { .. } => DiffRowKind::Delete,
            Self::Change { .. } => DiffRowKind::Change,
        }
    }
}

pub fn aligned_rows<'a>(old: &'a str, new: &'a str) -> Vec<AlignedRow<'a>> {
    let old_lines = old.tokenize_lines();
    let new_lines = new.tokenize_lines();
    let diff = TextDiff::configure()
        .algorithm(Algorithm::Patience)
        .diff_slices(&old_lines, &new_lines);

    let mut rows = Vec::new();
    for operation in diff.ops() {
        match *operation {
            DiffOp::Equal {
                old_index,
                new_index,
                len,
            } => {
                for offset in 0..len {
                    rows.push(AlignedRow::Equal {
                        old: line(&old_lines, old_index + offset),
                        new: line(&new_lines, new_index + offset),
                    });
                }
            }
            DiffOp::Delete {
                old_index, old_len, ..
            } => {
                for offset in 0..old_len {
                    rows.push(AlignedRow::Delete {
                        old: line(&old_lines, old_index + offset),
                    });
                }
            }
            DiffOp::Insert {
                new_index, new_len, ..
            } => {
                for offset in 0..new_len {
                    rows.push(AlignedRow::Add {
                        new: line(&new_lines, new_index + offset),
                    });
                }
            }
            DiffOp::Replace {
                old_index,
                old_len,
                new_index,
                new_len,
            } => {
                let paired = old_len.min(new_len);
                for offset in 0..paired {
                    rows.push(AlignedRow::Change {
                        old: line(&old_lines, old_index + offset),
                        new: line(&new_lines, new_index + offset),
                    });
                }
                for offset in paired..old_len {
                    rows.push(AlignedRow::Delete {
                        old: line(&old_lines, old_index + offset),
                    });
                }
                for offset in paired..new_len {
                    rows.push(AlignedRow::Add {
                        new: line(&new_lines, new_index + offset),
                    });
                }
            }
        }
    }
    rows
}

fn line<'a>(lines: &[&'a str], index: usize) -> DiffLine<'a> {
    DiffLine {
        number: index + 1,
        text: lines[index].trim_end_matches(['\r', '\n']),
    }
}

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
    use crate::model::{ItemKind, ProjectedItem, SourceSpan, SupportedPath};

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path = SupportedPath::new(RepoPath::new(path).expect("valid test path"))
            .expect("supported test path");
        ProjectedFile::try_new(
            path,
            vec![ProjectedItem {
                stable_key: "item".to_owned(),
                parent_key: None,
                kind: ItemKind::Function,
                name: "item".to_owned(),
                span: SourceSpan::new(0, 0, 0, 0, 0, 0),
                canonical_text: text.to_owned(),
            }],
        )
        .expect("valid fixture")
    }

    #[test]
    fn file_diff_path_agrees_with_each_present_side() {
        let old = projected("src/lib.rs", "fn a() {}\n");
        let new = projected("src/lib.rs", "fn a() -> u8 {}\n");
        let cases = [
            FileDiff::Added { new: new.clone() },
            FileDiff::Deleted { old: old.clone() },
            FileDiff::Modified { old, new },
        ];
        for diff in &cases {
            match diff {
                FileDiff::Added { new } => assert_eq!(diff.path(), new.path()),
                FileDiff::Deleted { old } => assert_eq!(diff.path(), old.path()),
                FileDiff::Modified { old, new } => {
                    assert_eq!(diff.path(), old.path());
                    assert_eq!(old.path(), new.path());
                }
            }
        }
    }

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

    #[test]
    fn aligned_rows_of_equal_texts_are_all_equal() {
        let rows = aligned_rows("a\nb\n", "a\nb\n");
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|row| row.kind() == DiffRowKind::Equal));

        let AlignedRow::Equal { old, .. } = rows[0] else {
            panic!("expected an equal row, got {:?}", rows[0]);
        };
        assert_eq!(old.number, 1);

        let AlignedRow::Equal { new, .. } = rows[1] else {
            panic!("expected an equal row, got {:?}", rows[1]);
        };
        assert_eq!(new.number, 2);
        assert_eq!(new.text, "b");
    }

    #[test]
    fn aligned_rows_of_empty_texts_is_empty() {
        assert!(aligned_rows("", "").is_empty());
    }

    #[test]
    fn aligned_rows_marks_an_insertion() {
        let rows = aligned_rows("a\nc\n", "a\nb\nc\n");
        assert_eq!(
            rows.iter().map(|row| row.kind()).collect::<Vec<_>>(),
            vec![DiffRowKind::Equal, DiffRowKind::Add, DiffRowKind::Equal]
        );

        let AlignedRow::Add { new } = rows[1] else {
            panic!("expected an added row, got {:?}", rows[1]);
        };
        assert_eq!(new.number, 2);
        assert_eq!(new.text, "b");
    }

    #[test]
    fn aligned_rows_marks_a_deletion() {
        let rows = aligned_rows("a\nb\nc\n", "a\nc\n");
        assert_eq!(rows[1].kind(), DiffRowKind::Delete);

        let AlignedRow::Delete { old } = rows[1] else {
            panic!("expected a deleted row, got {:?}", rows[1]);
        };
        assert_eq!(old.text, "b");
    }

    #[test]
    fn aligned_rows_marks_a_replacement_as_change() {
        let rows = aligned_rows("a\nb\nc\n", "a\nB\nc\n");
        assert_eq!(rows[1].kind(), DiffRowKind::Change);

        let AlignedRow::Change { old, new } = rows[1] else {
            panic!("expected a changed row, got {:?}", rows[1]);
        };
        assert_eq!(old.text, "b");
        assert_eq!(new.text, "B");
    }

    #[test]
    fn aligned_rows_pads_an_uneven_replacement() {
        let rows = aligned_rows("a\nb\nc\nd\n", "a\nX\n");
        let kinds: Vec<DiffRowKind> = rows.iter().map(|row| row.kind()).collect();
        assert_eq!(
            kinds,
            vec![
                DiffRowKind::Equal,
                DiffRowKind::Change,
                DiffRowKind::Delete,
                DiffRowKind::Delete
            ]
        );

        let AlignedRow::Change { old, new } = rows[1] else {
            panic!("expected a changed row, got {:?}", rows[1]);
        };
        assert_eq!(old.text, "b");
        assert_eq!(new.text, "X");
    }

    #[test]
    fn aligned_rows_handles_missing_sides() {
        let added = aligned_rows("", "x\n");
        assert_eq!(added.len(), 1);
        let AlignedRow::Add { new } = added[0] else {
            panic!("expected an added row, got {:?}", added[0]);
        };
        assert_eq!(new.number, 1);

        let deleted = aligned_rows("x\n", "");
        assert_eq!(deleted.len(), 1);
        let AlignedRow::Delete { old } = deleted[0] else {
            panic!("expected a deleted row, got {:?}", deleted[0]);
        };
        assert_eq!(old.number, 1);
    }

    #[test]
    fn aligned_rows_agree_with_unified_hunks_on_changed_lines() {
        let cases = [
            ("", ""),
            ("", "a\n"),
            ("a\n", ""),
            ("a\nb\nc\n", "a\nb\nc\n"),
            ("a\nb\nc\n", "a\nc\n"),
            ("a\nc\n", "a\nb\nc\n"),
            ("a\nb\nc\n", "a\nB\nc\n"),
            ("a\nb\nc\nd\n", "a\nX\n"),
            ("a\nX\n", "a\nb\nc\nd\n"),
            ("one\ntwo\n", "one\nTWO\nthree\n"),
        ];

        for (old, new) in cases {
            let rows = aligned_rows(old, new);
            let old_changed: Vec<usize> = rows
                .iter()
                .filter_map(|row| match row {
                    AlignedRow::Delete { old } | AlignedRow::Change { old, .. } => Some(old.number),
                    AlignedRow::Equal { .. } | AlignedRow::Add { .. } => None,
                })
                .collect();
            let new_changed: Vec<usize> = rows
                .iter()
                .filter_map(|row| match row {
                    AlignedRow::Add { new } | AlignedRow::Change { new, .. } => Some(new.number),
                    AlignedRow::Equal { .. } | AlignedRow::Delete { .. } => None,
                })
                .collect();

            let (unified_old, unified_new) = unified_changed_lines(&unified_hunks(old, new));
            assert_eq!(
                old_changed, unified_old,
                "old side differs for {old:?} -> {new:?}"
            );
            assert_eq!(
                new_changed, unified_new,
                "new side differs for {old:?} -> {new:?}"
            );
        }
    }

    fn unified_changed_lines(hunks: &str) -> (Vec<usize>, Vec<usize>) {
        let mut old_changed = Vec::new();
        let mut new_changed = Vec::new();
        let mut old_line = 0;
        let mut new_line = 0;

        for line in hunks.lines() {
            if let Some(header) = line.strip_prefix("@@ ") {
                let header = header.trim_end_matches(" @@");
                let mut ranges = header.split(' ');
                old_line = ranges
                    .next()
                    .and_then(|range| range.trim_start_matches('-').split(',').next())
                    .and_then(|start| start.parse().ok())
                    .unwrap_or(0);
                new_line = ranges
                    .next()
                    .and_then(|range| range.trim_start_matches('+').split(',').next())
                    .and_then(|start| start.parse().ok())
                    .unwrap_or(0);
                continue;
            }
            match line.as_bytes().first() {
                Some(b' ') => {
                    old_line += 1;
                    new_line += 1;
                }
                Some(b'-') => {
                    old_changed.push(old_line);
                    old_line += 1;
                }
                Some(b'+') => {
                    new_changed.push(new_line);
                    new_line += 1;
                }
                _ => {}
            }
        }

        (old_changed, new_changed)
    }
}
