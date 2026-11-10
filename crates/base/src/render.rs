//! Canonical rendering of project and file framing, independent of terminal
//! width for deterministic `show` and `diff` output.
//!
//! This module is Git-agnostic by design: it operates on already-produced
//! [`ProjectedFile`] values and [`FileDiff`] events and never on repository or
//! Tree-sitter types. The Git-aware pipeline that supplies those projections
//! lives in the CLI (TECHNICAL_DESIGN.md section 3).
//!
//! [`project_all`] is all-or-nothing: one file's projection failure aborts the
//! whole call, and callers must supply projectors in a fixed order so selection
//! is deterministic. A diff block is emitted only when a path's canonical text
//! differs, except for a moved file: a move keeps its rename headers even when
//! the two focused projections are equal, so a pure move stays visible.

use crate::diagnostic::ProjectionError;
use crate::diff::{FileDiff, unified_hunks};
use crate::language::{LanguageProjector, ProjectionInput};
use crate::model::{ProjectedFile, ProjectionMode, RepoPath, SupportedPath};

pub const LINE_WIDTH: usize = 80;

pub fn select_projector<'a>(
    projectors: &'a [&'a dyn LanguageProjector],
    path: &RepoPath,
) -> Option<&'a dyn LanguageProjector> {
    projectors
        .iter()
        .copied()
        .find(|projector| projector.supports_path(path))
}

pub fn sort_files(files: &mut [ProjectedFile]) {
    files.sort_by(|left, right| left.path().cmp(right.path()));
}

pub fn project_all<'a, I>(
    projectors: &[&dyn LanguageProjector],
    mode: ProjectionMode,
    sources: I,
) -> Result<Vec<ProjectedFile>, ProjectionError>
where
    I: IntoIterator<Item = (&'a RepoPath, &'a str)>,
{
    let mut files = Vec::new();
    for (path, source) in sources {
        let Some(projector) = select_projector(projectors, path) else {
            continue;
        };
        let path = SupportedPath::new(path.clone())
            .expect("a selected projector implies a supported path");
        files.push(projector.project(ProjectionInput {
            path: &path,
            source,
            mode,
        })?);
    }
    sort_files(&mut files);
    Ok(files)
}

pub fn show_document(files: &[ProjectedFile]) -> String {
    let mut ordered: Vec<&ProjectedFile> = files.iter().collect();
    ordered.sort_by(|left, right| left.path().cmp(right.path()));

    let mut document = String::new();
    for file in ordered {
        if !document.is_empty() {
            document.push('\n');
        }
        document.push_str("== ");
        document.push_str(&file.path().to_string());
        document.push_str(" ==\n");
        document.push_str(file.canonical_text());
    }
    document
}

pub fn diff_document(diffs: &[FileDiff]) -> String {
    let mut ordered: Vec<&FileDiff> = diffs.iter().collect();
    // `FileDiff::path` is the target-side display/selection key: the new path
    // for an addition, the removed path for a deletion, and the destination
    // path for a move.
    ordered.sort_by(|left, right| left.path().cmp(right.path()));

    let mut document = String::new();
    for diff in ordered {
        match diff {
            FileDiff::Added { new } => {
                let new_text = new.canonical_text();
                if new_text.is_empty() {
                    continue;
                }
                let display = new.path().to_string();
                push_path_header(&mut document, &display, &display);
                document.push_str("--- /dev/null\n");
                document.push_str(&format!("+++ b/{display}\n"));
                document.push_str(&unified_hunks("", new_text));
            }
            FileDiff::Deleted { old } => {
                let old_text = old.canonical_text();
                if old_text.is_empty() {
                    continue;
                }
                let display = old.path().to_string();
                push_path_header(&mut document, &display, &display);
                document.push_str(&format!("--- a/{display}\n"));
                document.push_str("+++ /dev/null\n");
                document.push_str(&unified_hunks(old_text, ""));
            }
            FileDiff::Modified { old, new } => {
                let old_text = old.canonical_text();
                let new_text = new.canonical_text();
                if old_text == new_text {
                    continue;
                }
                let display = new.path().to_string();
                push_path_header(&mut document, &display, &display);
                document.push_str(&format!("--- a/{display}\n"));
                document.push_str(&format!("+++ b/{display}\n"));
                document.push_str(&unified_hunks(old_text, new_text));
            }
            FileDiff::Moved { old, new } => {
                let old_text = old.canonical_text();
                let new_text = new.canonical_text();
                // The engine omits this event, but the public renderer must not
                // print a move that has no content on either requested-mode
                // side.
                if old_text.is_empty() && new_text.is_empty() {
                    continue;
                }
                let old_display = old.path().to_string();
                let new_display = new.path().to_string();
                push_path_header(&mut document, &old_display, &new_display);
                document.push_str(&format!("rename from {old_display}\n"));
                document.push_str(&format!("rename to {new_display}\n"));

                if old_text != new_text {
                    document.push_str(&format!("--- a/{old_display}\n"));
                    document.push_str(&format!("+++ b/{new_display}\n"));
                    document.push_str(&unified_hunks(old_text, new_text));
                }
            }
        }
    }
    document
}

fn push_path_header(document: &mut String, old_display: &str, new_display: &str) {
    document.push_str(&format!("diff --codect a/{old_display} b/{new_display}\n"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKind, Language, ProjectedItem, SourceSpan};

    fn projected(path: &str, text: &str) -> ProjectedFile {
        projected_raw(path.as_bytes(), text)
    }

    fn projected_raw(path: &[u8], text: &str) -> ProjectedFile {
        let path = SupportedPath::new(RepoPath::new(path).unwrap()).unwrap();
        if text.is_empty() {
            return ProjectedFile::try_new(path, Vec::new()).expect("valid fixture");
        }

        let item = ProjectedItem {
            stable_key: "item".to_owned(),
            parent_key: None,
            kind: ItemKind::Function,
            name: "item".to_owned(),
            span: SourceSpan::new(0, 0, 0, 0, 0, 0),
            canonical_text: text.to_owned(),
        };
        ProjectedFile::try_new(path, vec![item]).expect("valid fixture")
    }

    #[test]
    fn sort_files_orders_by_raw_path_bytes() {
        let mut files = vec![
            projected("b.rs", "b\n"),
            projected("a/x.rs", "x\n"),
            projected("a.rs", "a\n"),
        ];
        sort_files(&mut files);
        let paths: Vec<String> = files.iter().map(|file| file.path().to_string()).collect();
        assert_eq!(paths, vec!["a.rs", "a/x.rs", "b.rs"]);
    }

    #[test]
    fn show_document_has_headers_in_byte_order_separated_by_one_blank_line() {
        let document = show_document(&[
            projected("b.rs", "pub fn b();\n"),
            projected("a.rs", "pub fn a();\n"),
        ]);
        assert_eq!(
            document,
            "== a.rs ==\npub fn a();\n\n== b.rs ==\npub fn b();\n"
        );
        assert!(document.ends_with('\n'));
        assert!(!document.ends_with("\n\n"));
    }

    #[test]
    fn show_document_of_no_files_is_empty() {
        assert_eq!(show_document(&[]), "");
    }

    #[test]
    fn show_document_handles_empty_canonical_text() {
        let document = show_document(&[projected("a.rs", ""), projected("b.rs", "b\n")]);
        assert_eq!(document, "== a.rs ==\n\n== b.rs ==\nb\n");
    }

    #[test]
    fn diff_document_is_empty_when_all_projections_match() {
        let same = FileDiff::Modified {
            old: projected("a.rs", "pub fn a();\n"),
            new: projected("a.rs", "pub fn a();\n"),
        };
        assert_eq!(diff_document(&[same]), "");
        assert_eq!(diff_document(&[]), "");
    }

    #[test]
    fn diff_document_marks_added_and_deleted_files_with_dev_null() {
        let diffs = vec![
            FileDiff::Added {
                new: projected("fresh.rs", "pub fn fresh();\n"),
            },
            FileDiff::Deleted {
                old: projected("gone.rs", "pub fn gone();\n"),
            },
        ];
        let document = diff_document(&diffs);

        assert_eq!(
            document,
            concat!(
                "diff --codect a/fresh.rs b/fresh.rs\n",
                "--- /dev/null\n",
                "+++ b/fresh.rs\n",
                "@@ -0,0 +1 @@\n",
                "+pub fn fresh();\n",
                "diff --codect a/gone.rs b/gone.rs\n",
                "--- a/gone.rs\n",
                "+++ /dev/null\n",
                "@@ -1 +0,0 @@\n",
                "-pub fn gone();\n",
            )
        );
    }

    #[test]
    fn diff_document_marks_a_modified_file() {
        let diffs = vec![FileDiff::Modified {
            old: projected("a.rs", "pub fn a() -> u32;\n"),
            new: projected("a.rs", "pub fn a() -> u64;\n"),
        }];
        let document = diff_document(&diffs);

        assert_eq!(
            document,
            concat!(
                "diff --codect a/a.rs b/a.rs\n",
                "--- a/a.rs\n",
                "+++ b/a.rs\n",
                "@@ -1 +1 @@\n",
                "-pub fn a() -> u32;\n",
                "+pub fn a() -> u64;\n",
            )
        );
    }

    #[test]
    fn diff_document_renders_a_pure_move_without_content_headers_or_hunks() {
        let diffs = vec![FileDiff::Moved {
            old: projected("src/old.rs", "pub fn same();\n"),
            new: projected("src/new.rs", "pub fn same();\n"),
        }];
        let document = diff_document(&diffs);

        assert_eq!(
            document,
            concat!(
                "diff --codect a/src/old.rs b/src/new.rs\n",
                "rename from src/old.rs\n",
                "rename to src/new.rs\n",
            )
        );
        assert!(!document.contains("--- "));
        assert!(!document.contains("+++ "));
        assert!(!document.contains("@@"));
    }

    #[test]
    fn diff_document_omits_a_move_with_two_empty_projections() {
        let diffs = vec![FileDiff::Moved {
            old: projected("src/old.rs", ""),
            new: projected("src/new.rs", ""),
        }];
        assert_eq!(diff_document(&diffs), "");
    }

    #[test]
    fn diff_document_renders_an_edited_move_with_both_sides_and_hunks() {
        let diffs = vec![FileDiff::Moved {
            old: projected("src/old.rs", "pub fn old() -> u32;\n"),
            new: projected("src/new.rs", "pub fn new() -> u64;\n"),
        }];
        let document = diff_document(&diffs);

        assert_eq!(
            document,
            concat!(
                "diff --codect a/src/old.rs b/src/new.rs\n",
                "rename from src/old.rs\n",
                "rename to src/new.rs\n",
                "--- a/src/old.rs\n",
                "+++ b/src/new.rs\n",
                "@@ -1 +1 @@\n",
                "-pub fn old() -> u32;\n",
                "+pub fn new() -> u64;\n",
            )
        );
    }

    #[test]
    fn diff_document_escapes_both_move_paths_like_same_path_headers() {
        let diffs = vec![FileDiff::Moved {
            old: projected_raw(b"src/\xFF/old.rs", "pub fn same();\n"),
            new: projected_raw(b"src/\xFE/new.rs", "pub fn same();\n"),
        }];
        let document = diff_document(&diffs);

        assert_eq!(
            document,
            concat!(
                "diff --codect a/src/\\xFF/old.rs b/src/\\xFE/new.rs\n",
                "rename from src/\\xFF/old.rs\n",
                "rename to src/\\xFE/new.rs\n",
            )
        );
    }

    #[test]
    fn diff_document_orders_blocks_by_target_path_bytes() {
        let diffs = vec![
            FileDiff::Modified {
                old: projected("b.rs", "old b\n"),
                new: projected("b.rs", "new b\n"),
            },
            FileDiff::Moved {
                old: projected("zz/old.rs", "same\n"),
                new: projected("a/renamed.rs", "same\n"),
            },
            FileDiff::Modified {
                old: projected("a.rs", "old a\n"),
                new: projected("a.rs", "new a\n"),
            },
        ];
        let document = diff_document(&diffs);

        let renamed = document.find("b/a/renamed.rs").expect("renamed block");
        let a = document.find("a/a.rs").expect("a.rs block");
        let b = document.find("a/b.rs").expect("b.rs block");
        assert!(
            a < renamed && renamed < b,
            "blocks must be ordered by target path bytes: {document:?}"
        );
    }

    #[test]
    fn diff_document_treats_an_empty_projection_as_absent_content() {
        let diffs = vec![FileDiff::Added {
            new: projected("empty.rs", ""),
        }];
        assert_eq!(diff_document(&diffs), "");
    }

    #[test]
    fn rendered_documents_contain_no_escape_bytes() {
        let files = vec![projected("a.rs", "old\n")];
        let diffs = vec![
            FileDiff::Modified {
                old: projected("a.rs", "old\n"),
                new: projected("a.rs", "new\n"),
            },
            FileDiff::Moved {
                old: projected("old.rs", "same\n"),
                new: projected("new.rs", "same\n"),
            },
        ];
        assert!(!show_document(&files).contains('\u{1b}'));
        assert!(!diff_document(&diffs).contains('\u{1b}'));
    }

    fn span() -> SourceSpan {
        SourceSpan::new(0, 0, 0, 0, 0, 0)
    }

    struct MockProjector;

    impl LanguageProjector for MockProjector {
        fn language(&self) -> Language {
            Language::Rust
        }

        fn supports_path(&self, path: &RepoPath) -> bool {
            path.is_rust()
        }

        fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
            ProjectedFile::try_new(
                input.path.clone(),
                vec![ProjectedItem {
                    stable_key: "item".to_owned(),
                    parent_key: None,
                    kind: ItemKind::Function,
                    name: "item".to_owned(),
                    span: span(),
                    canonical_text: input.source.to_owned(),
                }],
            )
        }
    }

    struct FailingProjector;

    impl LanguageProjector for FailingProjector {
        fn language(&self) -> Language {
            Language::Rust
        }

        fn supports_path(&self, path: &RepoPath) -> bool {
            path.is_rust()
        }

        fn project(&self, input: ProjectionInput<'_>) -> Result<ProjectedFile, ProjectionError> {
            Err(ProjectionError::ErroneousSyntax {
                path: input.path.clone(),
                range: span(),
            })
        }
    }

    #[test]
    fn select_projector_matches_by_support_and_reports_unsupported_paths() {
        let projectors: [&dyn LanguageProjector; 1] = [&MockProjector];
        let rust = RepoPath::new("src/lib.rs").unwrap();
        let readme = RepoPath::new("README.md").unwrap();

        assert!(select_projector(&projectors, &rust).is_some());
        assert!(select_projector(&projectors, &readme).is_none());
    }

    #[test]
    fn project_all_skips_unsupported_paths_and_sorts_by_bytes() {
        let projectors: [&dyn LanguageProjector; 1] = [&MockProjector];
        let b = RepoPath::new("b.rs").unwrap();
        let a = RepoPath::new("a.rs").unwrap();
        let readme = RepoPath::new("README.md").unwrap();

        let files = project_all(
            &projectors,
            ProjectionMode::Types,
            [(&b, "b\n"), (&readme, "ignored\n"), (&a, "a\n")],
        )
        .unwrap();

        let paths: Vec<String> = files.iter().map(|file| file.path().to_string()).collect();
        assert_eq!(paths, vec!["a.rs", "b.rs"]);
    }

    #[test]
    fn project_all_propagates_projection_errors_fatally() {
        let projectors: [&dyn LanguageProjector; 1] = [&FailingProjector];
        let rust = RepoPath::new("src/lib.rs").unwrap();

        let error = project_all(&projectors, ProjectionMode::Types, [(&rust, "x\n")]).unwrap_err();
        assert_eq!(error.path(), &rust);
    }
}
