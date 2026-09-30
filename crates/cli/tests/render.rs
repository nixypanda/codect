//! Mixed-language project rendering and diff composition
//! (TECHNICAL_DESIGN.md sections 7.1, 7.2, 10.1, 16.2).
//!
//! These live in the CLI crate because it depends on core and both language
//! crates; placing them in core would introduce a dev-dependency cycle.

use std::fs;
use std::path::PathBuf;

use base::{
    LanguageProjector, ProjectedFile, ProjectionInput, ProjectionMode, RepoPath, SupportedPath,
    diff_document, show_document,
};
use language_elm::ElmProjector;
use language_haskell::HaskellProjector;
use language_python::PythonProjector;
use language_rust::RustProjector;

const MODES: [ProjectionMode; 2] = [ProjectionMode::Types, ProjectionMode::Signatures];

fn fixture(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn project(
    projector: &dyn LanguageProjector,
    path: &str,
    source: &str,
    mode: ProjectionMode,
) -> ProjectedFile {
    let path = SupportedPath::new(RepoPath::new(path).expect("fixture path is valid"))
        .expect("fixture path has a supported extension");
    projector
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .unwrap_or_else(|error| panic!("project {path}: {error}"))
}

#[test]
fn mixed_show_document_orders_elm_before_rust_in_both_modes() {
    let elm_source = fixture("elm/normal-module/input.elm");
    let rust_source = fixture("rust/structs/input.rs");

    for mode in MODES {
        let elm = project(&ElmProjector, "src/Main.elm", &elm_source, mode);
        let rust = project(&RustProjector, "src/lib.rs", &rust_source, mode);

        // Passed Rust first to prove the document reorders by raw path bytes
        // ("src/Main.elm" < "src/lib.rs" because 'M' < 'l').
        let document = show_document(&[rust.clone(), elm.clone()]);

        let expected = format!(
            "== src/Main.elm ==\n{}\n== src/lib.rs ==\n{}",
            elm.canonical_text(),
            rust.canonical_text()
        );
        assert_eq!(document, expected, "unexpected {mode:?} show document");
        assert!(
            !document.contains('\u{1b}'),
            "core rendering must be plain text"
        );
    }
}

#[test]
fn mixed_diff_document_covers_added_deleted_modified_and_unchanged_files() {
    let elm_old = fixture("elm/parameterized-types/input.elm");
    let elm_modified = fixture("elm/normal-module/input.elm");
    let elm_added = fixture("elm/port-module/input.elm");
    let rust_unchanged = fixture("rust/structs/input.rs");
    let rust_deleted = fixture("rust/unions-aliases/input.rs");

    for mode in MODES {
        let elm = ElmProjector;
        let rust = RustProjector;

        let old = vec![
            project(&elm, "src/Main.elm", &elm_old, mode),
            project(&rust, "src/lib.rs", &rust_unchanged, mode),
            project(&rust, "src/Old.rs", &rust_deleted, mode),
        ];
        let new = vec![
            project(&elm, "src/App.elm", &elm_added, mode),
            project(&elm, "src/Main.elm", &elm_modified, mode),
            project(&rust, "src/lib.rs", &rust_unchanged, mode),
        ];

        let document = diff_document(&old, &new);
        assert!(
            !document.contains('\u{1b}'),
            "core rendering must be plain text"
        );

        assert!(
            document.contains(
                "diff --ownai a/src/App.elm b/src/App.elm\n--- /dev/null\n+++ b/src/App.elm\n"
            ),
            "added Elm file must use /dev/null: {document:?}"
        );
        assert!(
            document.contains(
                "diff --ownai a/src/Main.elm b/src/Main.elm\n--- a/src/Main.elm\n+++ b/src/Main.elm\n"
            ),
            "modified Elm file must emit both sides: {document:?}"
        );
        assert!(
            document.contains(
                "diff --ownai a/src/Old.rs b/src/Old.rs\n--- a/src/Old.rs\n+++ /dev/null\n"
            ),
            "deleted Rust file must use /dev/null: {document:?}"
        );
        assert!(
            !document.contains("a/src/lib.rs"),
            "an unchanged projection must emit no block in {mode:?}"
        );

        let app = document.find("a/src/App.elm").expect("App.elm block");
        let main = document.find("a/src/Main.elm").expect("Main.elm block");
        let old_rs = document.find("a/src/Old.rs").expect("Old.rs block");
        assert!(
            app < main && main < old_rs,
            "blocks must be in raw path byte order: {document:?}"
        );
    }
}

#[test]
fn show_document_and_diff_document_are_empty_without_files() {
    assert_eq!(show_document(&[]), "");
    assert_eq!(diff_document(&[], &[]), "");
}

#[test]
fn show_document_orders_all_four_languages_by_raw_path_bytes() {
    let elm = fixture("elm/normal-module/input.elm");
    let haskell = fixture("haskell/canonical-types/input.hs");
    let python = fixture("python/canonical-types/input.py");
    let rust = fixture("rust/structs/input.rs");

    for mode in MODES {
        let elm = project(&ElmProjector, "src/App.elm", &elm, mode);
        let haskell = project(&HaskellProjector, "src/Main.hs", &haskell, mode);
        let python = project(&PythonProjector, "src/app.py", &python, mode);
        let rust = project(&RustProjector, "src/lib.rs", &rust, mode);

        // Supplied out of order to prove the document reorders by raw bytes:
        // "src/App.elm" < "src/Main.hs" < "src/app.py" < "src/lib.rs".
        let document = show_document(&[rust.clone(), python.clone(), haskell.clone(), elm.clone()]);

        let expected = format!(
            "== src/App.elm ==\n{}\n== src/Main.hs ==\n{}\n== src/app.py ==\n{}\n== src/lib.rs ==\n{}",
            elm.canonical_text(),
            haskell.canonical_text(),
            python.canonical_text(),
            rust.canonical_text()
        );
        assert_eq!(document, expected, "unexpected {mode:?} show document");
        assert!(!document.contains('\u{1b}'));
    }
}

#[test]
fn diff_document_reports_haskell_and_python_changes() {
    let haskell_old = fixture("haskell/records/input.hs");
    let haskell_new = fixture("haskell/gadt/input.hs");
    let python_old = fixture("python/enums/input.py");
    let python_new = fixture("python/decorators/input.py");

    for mode in MODES {
        let old = vec![
            project(&HaskellProjector, "src/Model.hs", &haskell_old, mode),
            project(&PythonProjector, "src/model.py", &python_old, mode),
        ];
        let new = vec![
            project(&HaskellProjector, "src/Model.hs", &haskell_new, mode),
            project(&PythonProjector, "src/model.py", &python_new, mode),
        ];

        let document = diff_document(&old, &new);
        assert!(!document.contains('\u{1b}'));
        assert!(
            document.contains("diff --ownai a/src/Model.hs b/src/Model.hs\n"),
            "Haskell block missing in {mode:?}: {document:?}"
        );
        assert!(
            document.contains("diff --ownai a/src/model.py b/src/model.py\n"),
            "Python block missing in {mode:?}: {document:?}"
        );
    }
}
