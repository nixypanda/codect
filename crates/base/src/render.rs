//! Canonical rendering of project and file framing, independent of terminal
//! width for deterministic `show` and `diff` output.
//!
//! This module is Git-agnostic by design: it operates on already-produced
//! [`ProjectedFile`] values and never on repository or Tree-sitter types. The
//! Git-aware pipeline that supplies those projections lives in the CLI
//! (TECHNICAL_DESIGN.md section 3).

use crate::diagnostic::ProjectionError;
use crate::diff::unified_hunks;
use crate::language::{LanguageProjector, ProjectionInput};
use crate::model::{ProjectedFile, ProjectionMode, RepoPath, SupportedPath};

/// The canonical maximum display width of a rendered line.
///
/// Language renderers wrap a declaration list or arrow chain only when its flat
/// form would exceed this budget. The value is a compile-time constant, never a
/// terminal measurement, so output stays deterministic and identical regardless
/// of where it is displayed (TECHNICAL_DESIGN.md section 10).
pub const LINE_WIDTH: usize = 80;

/// Selects the projector that claims `path`, or `None` for an unsupported path.
///
/// Exclusions (unsupported extensions, for example) are silent, not errors
/// (TECHNICAL_DESIGN.md section 15). Callers must supply projectors in a fixed
/// order so selection does not depend on iteration order.
pub fn select_projector<'a>(
    projectors: &'a [&'a dyn LanguageProjector],
    path: &RepoPath,
) -> Option<&'a dyn LanguageProjector> {
    projectors
        .iter()
        .copied()
        .find(|projector| projector.supports_path(path))
}

/// Orders projected files by raw repository path bytes.
///
/// [`RepoPath`] already implements [`Ord`] over its raw bytes, which keeps
/// output deterministic even for paths that are not valid UTF-8 (section 5.1).
pub fn sort_files(files: &mut [ProjectedFile]) {
    files.sort_by(|left, right| left.path().cmp(right.path()));
}

/// Projects caller-supplied `(path, source)` pairs, skipping unsupported paths.
///
/// A [`ProjectionError`] aborts the whole call: a supported file that fails to
/// project must never yield a partial result (section 9). The returned files are
/// in raw path byte order.
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

/// Renders the `show` document (section 10.1):
///
/// ```text
/// == src/User.elm ==
/// <canonical projection>
/// ```
///
/// Files are ordered by raw path bytes and separated by exactly one blank line.
/// A document for zero files is empty; otherwise it ends with exactly one
/// trailing newline. Paths use [`RepoPath`]'s escaped `Display`.
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

/// Renders the `diff` document (section 10.1):
///
/// ```text
/// diff --codect a/src/User.elm b/src/User.elm
/// --- a/src/User.elm
/// +++ b/src/User.elm
/// <unified hunks>
/// ```
///
/// `old` and `new` are the two projections. A path present on only one side is
/// an added or deleted file and uses `/dev/null` for the absent `---`/`+++`
/// side; the `diff --codect` line keeps `a/` and `b/` labels, matching Git. Only
/// paths whose canonical text differs emit a block, so an all-equal comparison
/// (including a body-only change) produces empty output (section 7.2). Blocks
/// are concatenated in raw path byte order with no blank line between them, and
/// a non-empty document ends with exactly one trailing newline.
pub fn diff_document(old: &[ProjectedFile], new: &[ProjectedFile]) -> String {
    let mut paths: Vec<&RepoPath> = old
        .iter()
        .chain(new.iter())
        .map(ProjectedFile::path)
        .collect();
    paths.sort();
    paths.dedup();

    let mut document = String::new();
    for path in paths {
        let old_file = find_by_path(old, path);
        let new_file = find_by_path(new, path);
        let old_text = old_file.map_or("", ProjectedFile::canonical_text);
        let new_text = new_file.map_or("", ProjectedFile::canonical_text);
        if old_text == new_text {
            continue;
        }

        let display = path.to_string();
        document.push_str(&format!("diff --codect a/{display} b/{display}\n"));
        document.push_str(&format!(
            "--- {}\n",
            side_label(old_file.is_some(), 'a', &display)
        ));
        document.push_str(&format!(
            "+++ {}\n",
            side_label(new_file.is_some(), 'b', &display)
        ));
        document.push_str(&unified_hunks(old_text, new_text));
    }
    document
}

fn side_label(present: bool, prefix: char, display: &str) -> String {
    if present {
        format!("{prefix}/{display}")
    } else {
        "/dev/null".to_owned()
    }
}

fn find_by_path<'a>(files: &'a [ProjectedFile], path: &RepoPath) -> Option<&'a ProjectedFile> {
    files.iter().find(|file| file.path() == path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ItemKind, Language, ProjectedItem, SourceSpan};

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path = SupportedPath::new(RepoPath::new(path).unwrap()).unwrap();
        // An item whose fragment is empty still yields a bare "\n" canonical
        // text, so an empty projection is modeled with no items.
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
        let old = vec![projected("a.rs", "pub fn a();\n")];
        let new = vec![projected("a.rs", "pub fn a();\n")];
        assert_eq!(diff_document(&old, &new), "");
        assert_eq!(diff_document(&[], &[]), "");
    }

    #[test]
    fn diff_document_marks_added_and_deleted_files_with_dev_null() {
        let old = vec![projected("gone.rs", "pub fn gone();\n")];
        let new = vec![projected("fresh.rs", "pub fn fresh();\n")];
        let document = diff_document(&old, &new);

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
        let old = vec![projected("a.rs", "pub fn a() -> u32;\n")];
        let new = vec![projected("a.rs", "pub fn a() -> u64;\n")];
        let document = diff_document(&old, &new);

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
    fn diff_document_orders_blocks_by_raw_path_bytes() {
        let old = vec![projected("b.rs", "old b\n"), projected("a.rs", "old a\n")];
        let new = vec![projected("b.rs", "new b\n"), projected("a.rs", "new a\n")];
        let document = diff_document(&old, &new);

        let a = document.find("a/a.rs").expect("a.rs block");
        let b = document.find("a/b.rs").expect("b.rs block");
        assert!(a < b, "blocks must be in raw path byte order");
    }

    #[test]
    fn diff_document_treats_an_empty_projection_as_absent_content() {
        // A deleted file that projected to empty text is equal to its absent
        // counterpart, so it emits nothing.
        let old = vec![projected("empty.rs", "")];
        assert_eq!(diff_document(&old, &[]), "");
    }

    #[test]
    fn rendered_documents_contain_no_escape_bytes() {
        let old = vec![projected("a.rs", "old\n")];
        let new = vec![projected("a.rs", "new\n")];
        assert!(!show_document(&old).contains('\u{1b}'));
        assert!(!diff_document(&old, &new).contains('\u{1b}'));
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
