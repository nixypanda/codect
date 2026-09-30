//! Projection invariance and exclusion tests.

use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use language_haskell::HaskellProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("path")).expect("supported path")
}

fn project(source: &str, mode: ProjectionMode) -> String {
    let path = supported("src/Sample.hs");
    HaskellProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .expect("projection")
        .canonical_text()
        .to_owned()
}

fn both(source: &str) -> (String, String) {
    (
        project(source, ProjectionMode::Types),
        project(source, ProjectionMode::Signatures),
    )
}

#[test]
fn changing_only_a_function_body_leaves_both_projections_unchanged() {
    let base = "module M where\n\nfoo :: Int -> Int\nfoo x = x\n";
    let changed = "module M where\n\nfoo :: Int -> Int\nfoo x =\n  let y = x * 2\n  in y + 1\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn adding_a_deriving_clause_changes_the_projection() {
    let base = "data Color = Red | Green | Blue\n";
    let changed = "data  Color  =  Red | Green | Blue\n  deriving (Eq)\n";
    assert_ne!(both(base), both(changed));
}

#[test]
fn changing_comments_and_haddocks_leaves_both_projections_unchanged() {
    let base = "module M where\n\ndata S = S Int\n";
    let changed = "-- leading\nmodule M where\n\n-- | doc\n-- more\ndata S = S Int\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_whitespace_leaves_both_projections_unchanged() {
    let base = "foo :: Int -> Int\nfoo x = x\n";
    let changed = "foo   ::   Int   ->   Int\nfoo    x   =   x\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_a_type_changes_both_projections() {
    let base = "data S = S Int\n";
    let changed = "data S = S Bool\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_ne!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
}

#[test]
fn changing_a_signature_changes_only_signatures() {
    let base = "foo :: Int -> Int\nfoo x = x\n";
    let changed = "foo :: Int -> Bool\nfoo x = x\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_eq!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
}

#[test]
fn adding_an_unannotated_function_changes_only_signatures() {
    let base = "foo :: Int -> Int\nfoo x = x\n";
    let changed = "foo :: Int -> Int\nfoo x = x\n\nbar y = y\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_eq!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
    assert!(changed_signatures.contains("bar y"));
}

#[test]
fn a_class_default_body_is_invisible_but_the_signature_survives() {
    let base = "class C a where\n  m :: a -> Int\n  m _ = 0\n";
    let changed = "class C a where\n  m :: a -> Int\n  m _ = 99\n";
    assert_eq!(both(base), both(changed));
    assert!(project(base, ProjectionMode::Signatures).contains("m :: a -> Int"));
}

#[test]
fn an_instance_method_body_is_invisible_but_patterns_are_not() {
    let base = "instance C Int where\n  m x = x\n";
    let body_changed = "instance C Int where\n  m x = x + 1\n";
    let pattern_changed = "instance C Int where\n  m y = y\n";
    assert_eq!(both(base), both(body_changed));
    assert_ne!(both(base), both(pattern_changed));
}

#[test]
fn changing_a_pragma_changes_the_projection() {
    let base = "module M where\n";
    let changed = "{-# LANGUAGE GADTs #-}\nmodule M where\n";
    assert_ne!(both(base), both(changed));
}

#[test]
fn imports_are_excluded() {
    let source = "module M where\n\nimport Data.List (sort)\nimport qualified Data.Map as Map\n";
    let (types, signatures) = both(source);
    assert_eq!(types, "module M\n");
    assert_eq!(signatures, "module M\n");
}

#[test]
fn erroneous_syntax_is_fatal_instead_of_partial() {
    let path = supported("src/Sample.hs");
    let error = HaskellProjector
        .project(ProjectionInput {
            path: &path,
            source: "data S =\n",
            mode: ProjectionMode::Types,
        })
        .expect_err("erroneous syntax must fail");
    assert!(matches!(
        error,
        base::ProjectionError::ErroneousSyntax { .. }
    ));
}
