//! Projection-level acceptance gate for Phase 5 (TECHNICAL_DESIGN.md sections
//! 7.2, 16.3): body-only changes must be invisible, and type/signature changes
//! must surface only in their mode.

use ownai_core::{
    Language, LanguageProjector, ProjectedFile, ProjectionInput, ProjectionMode, RepoPath,
    diff_document,
};
use ownai_language_elm::ElmProjector;
use ownai_language_rust::RustProjector;

const MODES: [ProjectionMode; 2] = [ProjectionMode::Types, ProjectionMode::Signatures];

fn project(
    projector: &dyn LanguageProjector,
    path: &str,
    source: &str,
    mode: ProjectionMode,
) -> ProjectedFile {
    let path = RepoPath::new(path).expect("source path is valid");
    projector
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .unwrap_or_else(|error| panic!("project {path} in {mode:?}: {error}"))
}

/// Projects the same path from `base` and `variant` in one mode.
fn pair(
    projector: &dyn LanguageProjector,
    path: &str,
    base: &str,
    variant: &str,
    mode: ProjectionMode,
) -> (ProjectedFile, ProjectedFile) {
    (
        project(projector, path, base, mode),
        project(projector, path, variant, mode),
    )
}

fn assert_body_change_is_invisible(
    projector: &dyn LanguageProjector,
    language: Language,
    path: &str,
    base: &str,
    body_variant: &str,
) {
    for mode in MODES {
        let (before, after) = pair(projector, path, base, body_variant, mode);
        assert_eq!(
            before.canonical_text(),
            after.canonical_text(),
            "a {language:?} body-only change must not alter the {mode:?} projection"
        );
        assert_eq!(
            diff_document(&[before], &[after]),
            "",
            "a {language:?} body-only change must produce an empty focused diff in {mode:?}"
        );
    }
}

fn assert_signature_change_is_signatures_only(
    projector: &dyn LanguageProjector,
    language: Language,
    path: &str,
    base: &str,
    signature_variant: &str,
) {
    let (base_types, changed_types) = pair(
        projector,
        path,
        base,
        signature_variant,
        ProjectionMode::Types,
    );
    assert_eq!(
        base_types.canonical_text(),
        changed_types.canonical_text(),
        "a {language:?} signature change must not alter the Types projection"
    );
    assert_eq!(
        diff_document(&[base_types], &[changed_types]),
        "",
        "a {language:?} signature change must produce an empty Types diff"
    );

    let (base_signatures, changed_signatures) = pair(
        projector,
        path,
        base,
        signature_variant,
        ProjectionMode::Signatures,
    );
    assert_ne!(
        base_signatures.canonical_text(),
        changed_signatures.canonical_text(),
        "a {language:?} signature change must alter the Signatures projection"
    );
    assert!(
        !diff_document(&[base_signatures], &[changed_signatures]).is_empty(),
        "a {language:?} signature change must produce a focused Signatures diff"
    );
}

fn assert_type_change_is_visible_in_both_modes(
    projector: &dyn LanguageProjector,
    language: Language,
    path: &str,
    base: &str,
    type_variant: &str,
) {
    for mode in MODES {
        let (before, after) = pair(projector, path, base, type_variant, mode);
        assert_ne!(
            before.canonical_text(),
            after.canonical_text(),
            "a {language:?} type change must alter the {mode:?} projection"
        );
        assert!(
            !diff_document(&[before], &[after]).is_empty(),
            "a {language:?} type change must produce a focused {mode:?} diff"
        );
    }
}

const RUST_BASE: &str = r#"
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    format!("hi {name}")
}
"#;

const RUST_BODY_VARIANT: &str = r#"
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str) -> String {
    let upper = name.to_uppercase();
    format!("HI {upper}")
}
"#;

const RUST_SIGNATURE_VARIANT: &str = r#"
pub struct User {
    pub id: u32,
}

pub fn greet(name: &str, excited: bool) -> String {
    format!("hi {name}")
}
"#;

const RUST_TYPE_VARIANT: &str = r#"
pub struct User {
    pub id: u64,
}

pub fn greet(name: &str) -> String {
    format!("hi {name}")
}
"#;

const ELM_BASE: &str = r#"module User exposing (..)

type alias User =
    { name : String }

greet : String -> String
greet name =
    "hi " ++ name
"#;

const ELM_BODY_VARIANT: &str = r#"module User exposing (..)

type alias User =
    { name : String }

greet : String -> String
greet name =
    "hello " ++ name
"#;

const ELM_SIGNATURE_VARIANT: &str = r#"module User exposing (..)

type alias User =
    { name : String }

greet : String -> Int
greet name =
    "hi " ++ name
"#;

const ELM_TYPE_VARIANT: &str = r#"module User exposing (..)

type alias User =
    { name : String
    , id : Int
    }

greet : String -> String
greet name =
    "hi " ++ name
"#;

#[test]
fn rust_body_only_change_is_invisible() {
    assert_body_change_is_invisible(
        &RustProjector,
        Language::Rust,
        "src/lib.rs",
        RUST_BASE,
        RUST_BODY_VARIANT,
    );
}

#[test]
fn rust_signature_change_appears_only_in_signatures() {
    assert_signature_change_is_signatures_only(
        &RustProjector,
        Language::Rust,
        "src/lib.rs",
        RUST_BASE,
        RUST_SIGNATURE_VARIANT,
    );
}

#[test]
fn rust_type_change_appears_in_both_modes() {
    assert_type_change_is_visible_in_both_modes(
        &RustProjector,
        Language::Rust,
        "src/lib.rs",
        RUST_BASE,
        RUST_TYPE_VARIANT,
    );
}

#[test]
fn elm_body_only_change_is_invisible() {
    assert_body_change_is_invisible(
        &ElmProjector,
        Language::Elm,
        "src/User.elm",
        ELM_BASE,
        ELM_BODY_VARIANT,
    );
}

#[test]
fn elm_signature_change_appears_only_in_signatures() {
    assert_signature_change_is_signatures_only(
        &ElmProjector,
        Language::Elm,
        "src/User.elm",
        ELM_BASE,
        ELM_SIGNATURE_VARIANT,
    );
}

#[test]
fn elm_type_change_appears_in_both_modes() {
    assert_type_change_is_visible_in_both_modes(
        &ElmProjector,
        Language::Elm,
        "src/User.elm",
        ELM_BASE,
        ELM_TYPE_VARIANT,
    );
}

#[test]
fn signature_change_diff_names_the_changed_signature() {
    let (before, after) = pair(
        &RustProjector,
        "src/lib.rs",
        RUST_BASE,
        RUST_SIGNATURE_VARIANT,
        ProjectionMode::Signatures,
    );
    let document = diff_document(&[before], &[after]);

    assert!(
        document.contains("-pub fn greet(name: &str) -> String;"),
        "deletion side missing: {document:?}"
    );
    assert!(
        document.contains("+pub fn greet(name: &str, excited: bool) -> String;"),
        "insertion side missing: {document:?}"
    );
}
