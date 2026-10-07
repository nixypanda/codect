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

fn tests(source: &str) -> String {
    project(source, ProjectionMode::Tests)
}

#[test]
fn changing_only_a_test_body_leaves_the_tests_projection_unchanged() {
    let base = "def test_it_works() -> None:\n    assert True\n";
    let changed = "def test_it_works() -> None:\n    a = 1\n    b = 2\n    assert a + b == 3\n";
    assert_eq!(tests(base), tests(changed));
}

#[test]
fn changing_a_tests_mode_decorator_changes_the_projection() {
    // Decorators are preserved, so a test's own decorator is part of its
    // projected signature.
    let base = "@pytest.mark.parametrize(\"v\", [1])\ndef test_it(v: int) -> None: ...\n";
    let changed = "@pytest.mark.parametrize(\"v\", [1, 2])\ndef test_it(v: int) -> None: ...\n";
    assert_ne!(tests(base), tests(changed));
    assert!(
        tests(base).contains("@pytest.mark.parametrize(\"v\", [1])"),
        "{}",
        tests(base)
    );

    // Adding a decorator to a retained test, or to a class holding one, shows too.
    let plain = "def test_it() -> None: ...\n";
    let marked = "@pytest.mark.skip\ndef test_it() -> None: ...\n";
    assert_ne!(tests(plain), tests(marked));
    assert!(tests(marked).contains("@pytest.mark.skip"));

    let bare = "class TestThing:\n    def test_it(self) -> None: ...\n";
    let decorated =
        "@pytest.mark.usefixtures(\"db\")\nclass TestThing:\n    def test_it(self) -> None: ...\n";
    assert_ne!(tests(bare), tests(decorated));
    assert!(tests(decorated).contains("@pytest.mark.usefixtures(\"db\")"));
}

#[test]
fn adding_a_non_test_function_leaves_the_tests_projection_unchanged() {
    let base = "def test_it_works() -> None: ...\n";
    let changed = "def test_it_works() -> None: ...\n\n\ndef helper() -> None: ...\n";
    assert_eq!(tests(base), tests(changed));
    assert_ne!(
        project(base, ProjectionMode::Signatures),
        project(changed, ProjectionMode::Signatures)
    );
}

#[test]
fn renaming_a_test_changes_the_tests_projection() {
    let base = "def test_it_works() -> None: ...\n";
    let changed = "def test_it_also_works() -> None: ...\n";
    assert_ne!(tests(base), tests(changed));
    assert!(tests(changed).contains("def test_it_also_works() -> None: ..."));
}

#[test]
fn a_class_with_no_test_members_is_dropped_entirely() {
    // Without dropping the empty container, `container_doc` would render
    // `class Helper: ...` for every class in the file.
    let source = "class Helper:\n    def method(self) -> None: ...\n";
    assert_eq!(tests(source), "");
    assert!(project(source, ProjectionMode::Signatures).contains("class Helper"));
}

#[test]
fn a_test_class_keeps_its_header_and_only_its_test_members() {
    let source = "class TestThing:\n    def test_first(self) -> None: ...\n    def helper(self) -> None: ...\n";
    assert_eq!(
        tests(source),
        "class TestThing:\n    def test_first(self) -> None: ...\n"
    );
}

#[test]
fn a_decorated_test_is_kept_and_a_fixture_is_not() {
    let source = "@pytest.mark.parametrize(\"v\", [1])\ndef test_decorated(v: int) -> None: ...\n\n\
                  @pytest.fixture()\ndef a_fixture() -> int: ...\n";
    assert_eq!(
        tests(source),
        "@pytest.mark.parametrize(\"v\", [1])\ndef test_decorated(v: int) -> None: ...\n"
    );
}

#[test]
fn every_tests_mode_key_is_present_in_the_signatures_superset() {
    let source = "class Box:\n    value: int\n\ndef helper() -> None: ...\n\n\
                  def test_it_works() -> None: ...\n\n\
                  class TestThing:\n    def test_method(self) -> None: ...\n";
    let path = supported("src/sample.py");
    let projected = PythonProjector
        .project(ProjectionInput {
            path: &path,
            source,
            mode: ProjectionMode::Tests,
        })
        .expect("projection");
    let signatures = PythonProjector
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
