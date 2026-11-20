use base::{
    ItemKind, LanguageProjector, ProjectedItem, ProjectionInput, ProjectionMode, RepoPath,
    SupportedPath,
};
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

fn text(source: &str, mode: ProjectionMode) -> String {
    file(source, mode).canonical_text().to_owned()
}

// Group/branch keys share `::cond::`/`::branch::` prefixes with their members.
// Identify them by their terminal key segments (and synthetic Module kind), not
// by a broad substring that would also match member keys extending the prefix.
fn is_group(item: &ProjectedItem) -> bool {
    let parts: Vec<_> = item.stable_key.split("::").collect();
    item.kind == ItemKind::Module && parts.len() >= 2 && parts[parts.len() - 2] == "cond"
}

fn is_branch(item: &ProjectedItem) -> bool {
    let parts: Vec<_> = item.stable_key.split("::").collect();
    item.kind == ItemKind::Module
        && parts.len() >= 3
        && parts[parts.len() - 3] == "branch"
        && parts[parts.len() - 2].parse::<usize>().is_ok()
}

#[test]
fn conditional_group_branch_and_member_keys_match_the_contract_shape() {
    let source = "#if DEBUG\nstruct A {}\n#elseif RELEASE\n#else\nstruct B {}\n#endif\n";
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let projected = file(source, mode);
        let items = projected.items();

        let group = items
            .iter()
            .find(|item| item.stable_key.ends_with("::cond::if:DEBUG"))
            .unwrap_or_else(|| panic!("{mode:?}: missing group in {items:?}"));
        assert!(group.parent_key.is_none(), "{mode:?}: group must be a root");
        assert!(group.canonical_text.starts_with("#if DEBUG"));

        let branches: Vec<_> = items.iter().filter(|item| is_branch(item)).collect();
        assert_eq!(branches.len(), 3, "{mode:?}: all headers must survive");
        for suffix in [
            "::branch::0::if:DEBUG",
            "::branch::1::elseif:RELEASE",
            "::branch::2::else",
        ] {
            let branch = branches
                .iter()
                .find(|item| item.stable_key.ends_with(suffix))
                .unwrap_or_else(|| panic!("{mode:?}: missing branch {suffix}"));
            assert_eq!(
                branch.parent_key.as_deref(),
                Some(group.stable_key.as_str())
            );
        }

        let a = items
            .iter()
            .find(|item| item.name == "A")
            .expect("member A");
        let b = items
            .iter()
            .find(|item| item.name == "B")
            .expect("member B");
        assert!(
            a.parent_key
                .as_deref()
                .unwrap()
                .ends_with("::branch::0::if:DEBUG")
        );
        assert!(
            b.parent_key
                .as_deref()
                .unwrap()
                .ends_with("::branch::2::else")
        );
    }
}

#[test]
fn every_original_branch_header_is_preserved_when_any_branch_retains_a_member() {
    let source = "#if DEBUG\nstruct A {}\n#elseif RELEASE\n#else\nstruct B {}\n#endif\n";
    let expected = "#if DEBUG\nstruct A {}\n#elseif RELEASE\n#else\nstruct B {}\n#endif\n";
    assert_eq!(text(source, ProjectionMode::Types), expected);
    assert_eq!(text(source, ProjectionMode::Signatures), expected);

    let mixed = "#if DEBUG\nstruct A {}\n#else\nfunc f() {}\n#endif\n";
    for mode in [ProjectionMode::Types, ProjectionMode::Signatures] {
        let out = text(mixed, mode);
        assert!(out.contains("#if DEBUG"), "{out}");
        assert!(out.contains("#else"), "{out}");
        assert!(out.contains("#endif"), "{out}");
    }
    assert_eq!(
        text(mixed, ProjectionMode::Types),
        "#if DEBUG\nstruct A {}\n#else\n#endif\n"
    );
    assert_eq!(
        text(mixed, ProjectionMode::Signatures),
        "#if DEBUG\nstruct A {}\n#else\nfunc f() { ... }\n#endif\n"
    );
}

#[test]
fn a_conditional_group_with_no_retained_member_is_dropped_entirely() {
    let source = "#if A\nfunc a() {}\n#else\nfunc b() {}\n#endif\n";
    assert_eq!(text(source, ProjectionMode::Types), "");
    assert!(!text(source, ProjectionMode::Types).contains("#if"));
    assert!(text(source, ProjectionMode::Signatures).contains("func a() { ... }"));
}

#[test]
fn branch_spans_are_mode_independent_and_include_dropped_source_members() {
    let source = "#if DEBUG\nstruct A {}\n#else\nfunc f() {}\n#endif\n";
    let branch_spans = |mode| {
        file(source, mode)
            .items()
            .iter()
            .filter(|item| is_branch(item))
            .map(|item| {
                (
                    item.stable_key.clone(),
                    (item.span.start_byte(), item.span.end_byte()),
                )
            })
            .collect::<std::collections::BTreeMap<_, _>>()
    };
    let types = branch_spans(ProjectionMode::Types);
    let signatures = branch_spans(ProjectionMode::Signatures);
    assert_eq!(types.len(), 2, "{types:?}");
    assert_eq!(types, signatures, "branch spans must not depend on mode");

    // The #else branch is header-only in Types, but its span still ends at the
    // last source member (`func f() {}`), not at the #else header.
    let else_end = source.find("func f() {}").expect("callable") + "func f() {}".len();
    let else_span = types
        .iter()
        .find(|(key, _)| key.contains("::branch::1::"))
        .map(|(_, span)| *span)
        .expect("else branch");
    assert_eq!(else_span.1, else_end, "{else_span:?}");
}

#[test]
fn a_nested_conditional_group_is_parented_to_the_enclosing_branch() {
    let source = "#if A\n#if B\nstruct X {}\n#endif\n#endif\n";
    let projected = file(source, ProjectionMode::Signatures);
    let outer_branch = projected
        .items()
        .iter()
        .find(|item| is_branch(item) && item.stable_key.ends_with("::branch::0::if:A"))
        .expect("the outer branch");
    let groups: Vec<_> = projected
        .items()
        .iter()
        .filter(|item| is_group(item))
        .collect();
    assert_eq!(groups.len(), 2, "outer and nested groups: {groups:?}");
    assert!(
        groups
            .iter()
            .any(|group| group.parent_key.as_deref() == Some(outer_branch.stable_key.as_str())),
        "the nested group must hang off the outer branch: {groups:?}"
    );
}
