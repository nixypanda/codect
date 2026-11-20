use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_swift::SwiftProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("path")).expect("supported path")
}

fn file(source: &str, mode: ProjectionMode) -> base::ProjectedFile {
    let path = supported("src/lib.swift");
    SwiftProjector::new()
        .project(ProjectionInput {
            path: &path,
            source,
            mode,
        })
        .expect("projection")
}

#[test]
fn every_item_span_lies_within_the_source_and_is_nonempty() {
    let source = "@foo(bar: 1)\nstruct S {\n    var x: Int = 1\n    func f() {}\n}\n#if DEBUG\nlet g: Int = 2\n#endif\n@Test func t() {}\n";
    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        let projected = file(source, mode);
        if mode == ProjectionMode::Tests {
            assert!(
                !projected.items().is_empty(),
                "Tests mode must not be vacuous in this fixture"
            );
        }
        for item in projected.items() {
            let span = item.span;
            assert!(
                span.start_byte() < span.end_byte(),
                "{} in {mode:?} has an empty span",
                item.stable_key
            );
            assert!(
                source
                    .as_bytes()
                    .get(span.start_byte()..span.end_byte())
                    .is_some(),
                "{} in {mode:?} has an out-of-range span {:?}",
                item.stable_key,
                (span.start_byte(), span.end_byte())
            );
        }
    }
}

#[test]
fn spans_cover_the_declared_item_text() {
    let source = "struct S {\n    var x: Int\n}\n";
    let projected = file(source, ProjectionMode::Signatures);
    let s = projected
        .items()
        .iter()
        .find(|item| item.name == "S")
        .expect("type item");
    let x = projected
        .items()
        .iter()
        .find(|item| item.name == "x")
        .expect("field item");
    let s_text = &source[s.span.start_byte()..s.span.end_byte()];
    let x_text = &source[x.span.start_byte()..x.span.end_byte()];
    assert!(s_text.contains("struct S"), "{s_text:?}");
    assert!(s_text.contains("var x"), "{s_text:?}");
    assert!(x_text.contains("var x"), "{x_text:?}");
}

#[test]
fn retained_item_spans_are_mode_independent() {
    let source = "struct S {\n    var typed: Int\n    func helper() {}\n}\nlet g: Int = 1\n";
    let spans = |mode| {
        file(source, mode)
            .items()
            .iter()
            .map(|item| {
                (
                    item.stable_key.clone(),
                    (item.span.start_byte(), item.span.end_byte()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let types = spans(ProjectionMode::Types);
    let signatures = spans(ProjectionMode::Signatures);
    assert!(
        !types.is_empty(),
        "Types must retain the container and field"
    );
    for (key, span) in &types {
        assert_eq!(
            signatures.get(key),
            Some(span),
            "span for {key} differs between Types and Signatures"
        );
    }
}

#[test]
fn parent_chains_are_valid_and_acyclic_in_every_mode() {
    let source = "struct Outer {\n    struct Inner { let x: Int }\n    func f() {}\n}\n#if D\nlet g: Int = 1\n#endif\n@Test func t() {}\n";
    for mode in [
        ProjectionMode::Types,
        ProjectionMode::Signatures,
        ProjectionMode::Tests,
    ] {
        let projected = file(source, mode);
        let items = projected.items();
        if mode == ProjectionMode::Tests {
            assert!(!items.is_empty(), "Tests mode must not be vacuous here");
        }
        for item in items {
            let mut current = item.parent_key.as_deref();
            let mut visited = std::collections::HashSet::new();
            let mut steps = 0;
            while let Some(parent) = current {
                assert!(visited.insert(parent), "cyclic parent chain at {parent}");
                let container = items
                    .iter()
                    .find(|candidate| candidate.stable_key == parent)
                    .unwrap_or_else(|| panic!("{mode:?}: unknown parent {parent}"));
                assert_ne!(container.stable_key, item.stable_key, "self parent");
                current = container.parent_key.as_deref();
                steps += 1;
                assert!(steps <= items.len(), "parent chain did not terminate");
            }
        }
    }
}

#[test]
fn tests_mode_parent_chains_and_spans_through_nested_conditional_suites() {
    let source = "#if DEBUG\n@Suite struct Suite {\n    @Test func nested() {}\n}\n#else\n@Test func other() {}\n#endif\n";
    let projected = file(source, ProjectionMode::Tests);
    let items = projected.items();
    assert!(
        items.len() >= 5,
        "Tests must retain the group, both branches, the suite, and the tests: {items:?}"
    );
    assert!(
        items.iter().any(|item| item.parent_key.is_some()),
        "nested test items are required for a non-vacuous chain"
    );

    for item in items {
        assert!(
            item.span.start_byte() < item.span.end_byte(),
            "{} has an empty span",
            item.stable_key
        );
        assert!(
            source
                .as_bytes()
                .get(item.span.start_byte()..item.span.end_byte())
                .is_some(),
            "{} has an out-of-range span",
            item.stable_key
        );
    }

    let nested = items
        .iter()
        .find(|item| item.name.contains("nested"))
        .expect("the nested @Test method");
    let mut chain = Vec::new();
    let mut current = nested.parent_key.as_deref();
    while let Some(parent) = current {
        let container = items
            .iter()
            .find(|candidate| candidate.stable_key == parent)
            .expect("every parent must resolve to an item");
        chain.push(parent.to_owned());
        current = container.parent_key.as_deref();
    }
    assert!(
        chain.iter().any(|key| key.contains("::type::")),
        "the @Suite container must be in the chain: {chain:?}"
    );
    assert!(
        chain.iter().any(|key| key.contains("::branch::")),
        "the conditional branch must be in the chain: {chain:?}"
    );
    assert!(
        chain.iter().any(|key| key.contains("::cond::")),
        "the conditional group must be in the chain: {chain:?}"
    );

    // Member spans exclude attributes, but must still cover the declaration.
    let text = &source[nested.span.start_byte()..nested.span.end_byte()];
    assert!(
        text.contains("func") && text.contains("nested"),
        "unexpected span text {text:?}"
    );
}

#[test]
fn every_nested_item_reaches_a_top_level_ancestor() {
    let source =
        "struct Outer {\n    struct Inner { let x: Int }\n    enum Kind { case a, b }\n}\n";
    let projected = file(source, ProjectionMode::Signatures);
    assert!(
        projected
            .items()
            .iter()
            .any(|item| item.parent_key.is_some()),
        "the fixture must exercise nested declarations"
    );
    for item in projected.items() {
        let mut current = item.parent_key.as_deref();
        while let Some(parent) = current {
            current = projected
                .items()
                .iter()
                .find(|candidate| candidate.stable_key == parent)
                .expect("parent must resolve")
                .parent_key
                .as_deref();
        }
    }
}
