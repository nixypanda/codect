//! Projection invariance and exclusion tests (TECHNICAL_DESIGN.md sections 16.3
//! and 12.1).

use ownai_core::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use ownai_language_rust::RustProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("path")).expect("supported path")
}

fn project(source: &str, mode: ProjectionMode) -> String {
    let path = supported("src/lib.rs");
    RustProjector
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
    let base = "pub fn compute(a: u32) -> u32 { a }\n";
    let changed = "pub fn compute(a: u32) -> u32 {\n    let b = a * 2;\n    b + 1\n}\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_comments_leaves_both_projections_unchanged() {
    let base = "pub struct S { pub a: u32 }\n";
    let changed =
        "// leading\n/// documented\npub struct S { /* inline */ pub a: u32 } // trailing\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_whitespace_leaves_both_projections_unchanged() {
    let base = "pub struct S{pub a:u32,b:String}\n";
    let changed = "pub   struct   S\n{\n\tpub a : u32 ,\n    b : String ,\n}\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_a_type_changes_both_projections() {
    let base = "pub struct S { pub a: u32 }\n";
    let changed = "pub struct S { pub a: u64 }\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_ne!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
}

#[test]
fn changing_a_function_signature_changes_only_signatures() {
    let base = "pub fn f(a: u32) -> u32 { a }\n";
    let changed = "pub fn f(a: u32) -> u64 { a }\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_eq!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
}

#[test]
fn adding_a_private_function_changes_only_signatures() {
    let base = "pub fn visible() {}\n";
    let changed = "pub fn visible() {}\nfn hidden() {}\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_eq!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
    assert!(changed_signatures.contains("fn hidden();"));
}

#[test]
fn reordering_declarations_changes_projected_order() {
    let base = "pub struct A;\npub struct B;\n";
    let changed = "pub struct B;\npub struct A;\n";
    let base_types = project(base, ProjectionMode::Types);
    let changed_types = project(changed, ProjectionMode::Types);
    assert_ne!(base_types, changed_types);
    assert!(base_types.find("struct A").unwrap() < base_types.find("struct B").unwrap());
    assert!(changed_types.find("struct B").unwrap() < changed_types.find("struct A").unwrap());
}

#[test]
fn local_items_and_closures_are_excluded() {
    let source = "pub fn host() {\n    struct Local;\n    fn nested() {}\n    let closure = |x: i32| x + 1;\n}\n";
    let (types, signatures) = both(source);
    assert_eq!(types, "");
    assert!(!signatures.contains("Local"));
    assert!(!signatures.contains("nested"));
    assert_eq!(signatures, "pub fn host();\n");
}

#[test]
fn macro_definitions_and_invocations_produce_no_items() {
    let source = "macro_rules! m { () => { pub struct Generated; }; }\nm!();\n";
    let path = supported("src/lib.rs");
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let file = RustProjector
            .project(ProjectionInput {
                path: &path,
                source,
                mode,
            })
            .expect("macro declarations are not a projection error");
        assert!(file.items().is_empty());
        assert_eq!(file.canonical_text(), "");
    }
}

#[test]
fn derive_and_attribute_macros_are_not_expanded() {
    let source = "#[derive(Clone, Debug)]\n#[my_attribute(option = \"value\")]\npub struct S;\n";
    let types = project(source, ProjectionMode::Types);
    assert_eq!(
        types,
        "#[derive(Clone, Debug)]\n#[my_attribute(option = \"value\")]\npub struct S;\n"
    );
    assert!(!types.contains("impl"));
    assert_eq!(types, project(source, ProjectionMode::Signatures));
}

#[test]
fn erroneous_syntax_is_fatal_instead_of_partial() {
    let path = supported("src/lib.rs");
    let error = RustProjector
        .project(ProjectionInput {
            path: &path,
            source: "pub struct Broken {",
            mode: ProjectionMode::Signatures,
        })
        .expect_err("erroneous syntax must fail");
    assert!(matches!(
        error,
        ownai_core::ProjectionError::ErroneousSyntax { .. }
    ));
}

#[test]
fn every_nested_item_reaches_a_top_level_ancestor() {
    // A struct with a field produces a nested item whose `parent_key` names the
    // struct, so this fixture exercises the invariant `ProjectedFile::try_new`
    // now enforces.
    let source = "pub struct S {\n    pub a: u32,\n}\n";
    let path = supported("src/lib.rs");
    let file = RustProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Signatures,
        })
        .expect("projection");

    assert!(
        file.items().iter().any(|item| item.parent_key.is_some()),
        "the fixture must exercise a nested declaration"
    );
    for item in file.items() {
        let mut current = item.parent_key.as_deref();
        let mut steps = 0;
        while let Some(parent) = current {
            let container = file
                .items()
                .iter()
                .find(|candidate| candidate.stable_key == parent)
                .expect("every parent_key must resolve to an item");
            current = container.parent_key.as_deref();
            steps += 1;
            assert!(steps <= file.items().len(), "a parent chain must terminate");
        }
    }
}
