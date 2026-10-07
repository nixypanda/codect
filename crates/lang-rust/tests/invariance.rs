use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_rust::RustProjector;

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

fn tests(source: &str) -> String {
    project(source, ProjectionMode::Tests)
}

#[test]
fn changing_only_a_function_body_leaves_both_projections_unchanged() {
    let base = "pub fn compute(a: u32) -> u32 { a }\n";
    let changed = "pub fn compute(a: u32) -> u32 {\n    let b = a * 2;\n    b + 1\n}\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_only_a_test_body_leaves_the_tests_projection_unchanged() {
    let base = "#[test]\nfn it_works() {\n    assert!(true);\n}\n";
    let changed =
        "#[test]\nfn it_works() {\n    let a = 1;\n    let b = 2;\n    assert_eq!(a + b, 3);\n}\n";
    assert_eq!(tests(base), tests(changed));
}

#[test]
fn a_parametrized_test_attribute_is_detected_on_its_own() {
    // `test-case`'s `#[test_case]` and `#[test_matrix]` are primary markers,
    // used without `#[test]`. An entire module built from them must not vanish.
    let source =
        "#[test_case(-2, -4; \"both negative\")]\nfn multiplication_tests(x: i8, y: i8) {}\n";
    assert!(
        tests(source).contains("fn multiplication_tests(x: i8, y: i8);"),
        "{}",
        tests(source)
    );

    let matrix = "#[test_matrix([-2, 2], [-4, 4])]\nfn cartesian(x: i8, y: i8) {}\n";
    assert!(
        tests(matrix).contains("fn cartesian(x: i8, y: i8);"),
        "{}",
        tests(matrix)
    );

    // rstest's `#[case]` is the companion of `#[rstest]` and must not promote a
    // function on its own.
    let companion_only = "#[case(1, 2)]\nfn ordinary(a: u32, b: u32) -> u32 { a + b }\n";
    assert_eq!(tests(companion_only), "");

    let should_panic_only = "#[should_panic]\nfn ordinary() {}\n";
    assert_eq!(tests(should_panic_only), "");
}

#[test]
fn changing_a_test_attribute_changes_the_tests_projection() {
    // Non-doc attributes are preserved, so a test's own attribute is part of its
    // projected signature.
    let base = "#[rstest]\n#[case(1, 2)]\nfn adds(a: u32, b: u32) -> u32 { a + b }\n";
    let changed = "#[rstest]\n#[case(1, 3)]\nfn adds(a: u32, b: u32) -> u32 { a + b }\n";
    assert_ne!(tests(base), tests(changed));
    assert!(tests(base).contains("#[case(1, 2)]"), "{}", tests(base));

    // Adding a companion attribute to a test is likewise visible.
    let plain = "#[test]\nfn panics() {}\n";
    let with_companion = "#[test]\n#[should_panic]\nfn panics() {}\n";
    assert_ne!(tests(plain), tests(with_companion));
    assert!(tests(with_companion).contains("#[should_panic]"));
}

#[test]
fn adding_a_non_test_function_leaves_the_tests_projection_unchanged() {
    let base = "#[test]\nfn it_works() {}\n";
    let changed = "#[test]\nfn it_works() {}\n\npub fn helper() {}\n";
    assert_eq!(tests(base), tests(changed));
    assert_ne!(
        project(base, ProjectionMode::Signatures),
        project(changed, ProjectionMode::Signatures)
    );
}

#[test]
fn renaming_a_test_changes_the_tests_projection() {
    let base = "#[test]\nfn it_works() {}\n";
    let changed = "#[test]\nfn it_also_works() {}\n";
    assert_ne!(tests(base), tests(changed));
    assert!(tests(changed).contains("fn it_also_works();"));
}

#[test]
fn changing_a_tests_mode_declaration_type_is_invisible_because_types_are_dropped() {
    let base = "pub struct S {\n    pub a: u32,\n}\n\n#[test]\nfn it_works() {}\n";
    let changed = "pub struct S {\n    pub a: u64,\n}\n\n#[test]\nfn it_works() {}\n";
    // A tests projection declares no types, so a type change is invisible.
    assert_eq!(tests(base), tests(changed));
    assert_ne!(
        project(base, ProjectionMode::Types),
        project(changed, ProjectionMode::Types)
    );
}

#[test]
fn every_tests_mode_key_is_present_in_the_signatures_superset() {
    let source = "pub struct S;\n\npub fn helper() {}\n\n#[test]\nfn it_works() {}\n\n\
                  #[cfg(test)]\nmod tests {\n    #[test]\n    fn nested() {}\n}\n";
    let path = supported("src/lib.rs");
    let projected = RustProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Tests,
        })
        .expect("projection");
    let signatures = RustProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Signatures,
        })
        .expect("projection");

    assert!(projected.items().iter().all(|item| {
        signatures
            .items()
            .iter()
            .any(|other| other.stable_key == item.stable_key)
    }));
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
        base::ProjectionError::ErroneousSyntax { .. }
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
