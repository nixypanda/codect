use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_python::PythonProjector;

fn supported(raw: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(raw).expect("path")).expect("supported path")
}

fn project(source: &str, mode: ProjectionMode) -> String {
    let path = supported("src/sample.py");
    PythonProjector
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
    let base = "def compute(a: int) -> int:\n    return a\n";
    let changed = "def compute(a: int) -> int:\n    b = a * 2\n    return b + 1\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_a_value_initializer_leaves_both_projections_unchanged() {
    let base = "MAX: int = 1\nTOTAL = 2\n";
    let changed = "MAX: int = 999\nTOTAL = 500\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_comments_leaves_both_projections_unchanged() {
    let base = "class Box:\n    value: int\n";
    let changed = "# leading\nclass Box:\n    \"\"\"doc\"\"\"\n    value: int  # trailing\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_whitespace_leaves_both_projections_unchanged() {
    let base = "def add(a: int, b: int) -> int:\n    return a + b\n";
    let changed = "def add( a : int , b : int )->int :\n    return a+b\n";
    assert_eq!(both(base), both(changed));
}

#[test]
fn changing_a_type_changes_both_projections() {
    let base = "class Box:\n    value: int\n";
    let changed = "class Box:\n    value: str\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_ne!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
}

#[test]
fn changing_a_function_signature_changes_only_signatures() {
    let base = "def f(a: int) -> int:\n    return a\n";
    let changed = "def f(a: int) -> str:\n    return a\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_eq!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
}

#[test]
fn adding_a_private_function_changes_only_signatures() {
    let base = "def visible():\n    ...\n";
    let changed = "def visible():\n    ...\n\n\ndef hidden():\n    ...\n";
    let (base_types, base_signatures) = both(base);
    let (changed_types, changed_signatures) = both(changed);
    assert_eq!(base_types, changed_types);
    assert_ne!(base_signatures, changed_signatures);
    assert!(changed_signatures.contains("def hidden(): ..."));
}

#[test]
fn changing_a_decorator_changes_the_projection() {
    let base = "class Service:\n    def run(self) -> None: ...\n";
    let changed = "class Service:\n    @staticmethod\n    def run() -> None: ...\n";
    assert_ne!(both(base), both(changed));
}

#[test]
fn nested_functions_and_lambdas_are_excluded() {
    let source = "def outer():\n    def inner():\n        ...\n    class Local:\n        ...\n    return lambda x: x\n";
    let (types, signatures) = both(source);
    assert_eq!(types, "");
    assert_eq!(signatures, "def outer(): ...\n");
}

#[test]
fn imports_and_docstrings_produce_no_items() {
    let source = "\"\"\"doc\"\"\"\nimport os\nfrom typing import Any\n";
    let (types, signatures) = both(source);
    assert_eq!(types, "");
    assert_eq!(signatures, "");
}

#[test]
fn erroneous_syntax_is_fatal_instead_of_partial() {
    let path = supported("src/sample.py");
    let error = PythonProjector
        .project(ProjectionInput {
            path: &path,
            source: "def broken(:\n",
            mode: ProjectionMode::Types,
        })
        .expect_err("erroneous syntax must fail");
    assert!(matches!(
        error,
        base::ProjectionError::ErroneousSyntax { .. }
    ));
}
