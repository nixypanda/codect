//! Independent regressions for two renderer/extractor defects (review R1/R2).
//! All expected strings are written from the frozen contract and Swift source
//! semantics; none are captured from implementation output.

use base::{LanguageProjector, ProjectionInput, ProjectionMode, RepoPath, SupportedPath};
use lang_swift::SwiftProjector;

fn supported() -> SupportedPath {
    SupportedPath::new(RepoPath::new("src/lib.swift").unwrap()).unwrap()
}

fn project(source: &str, mode: ProjectionMode) -> base::ProjectedFile {
    SwiftProjector::new()
        .project(ProjectionInput {
            path: &supported(),
            source,
            mode,
        })
        .unwrap_or_else(|error| panic!("projection failed for {source:?}: {error}"))
}

fn text(source: &str, mode: ProjectionMode) -> String {
    project(source, mode).canonical_text().to_owned()
}

fn signatures(source: &str) -> String {
    text(source, ProjectionMode::Signatures)
}

fn types(source: &str) -> String {
    text(source, ProjectionMode::Types)
}

// --- R1: comparison operators are expressions, not generic brackets --------

#[test]
fn comparison_expression_default_keeps_operator_spacing_and_order() {
    let source = "func f(_ flag: Bool = a < b && c > d) {}\n";
    let expected = "func f(_ flag: Bool = a < b && c > d) { ... }\n";
    assert_eq!(types(source), "", "free functions are Signatures-only");
    assert_eq!(signatures(source), expected);
    assert!(
        !expected.contains(','),
        "no comma belongs in this expression"
    );
    assert!(!signatures(source).contains(','), "no generated comma");
}

#[test]
fn comparison_expression_in_an_attribute_keeps_operator_spacing_and_order() {
    let source = "@foo(v: 1 < 2 && 3 > 4)\nstruct S {}\n";
    let expected = "@foo(v: 1 < 2 && 3 > 4) struct S {}\n";
    assert_eq!(types(source), expected);
    assert_eq!(signatures(source), expected);
    assert!(!signatures(source).contains(','), "no generated comma");
}

#[test]
fn long_comparison_operands_wrap_without_an_invented_comma() {
    let expr = "someVeryLongIdentifierNameOne < someVeryLongIdentifierNameTwo && someVeryLongIdentifierNameThree > someVeryLongIdentifierNameFour";
    let source = format!("func f(_ flag: Bool = {expr}) {{}}\n");
    let out = signatures(&source);

    assert!(out.contains(expr), "expression changed: {out}");
    assert!(out.contains("func f(\n"), "long signature must wrap: {out}");
    assert!(out.contains("{ ... }"), "{out}");
    // The default is one atomic text token, so the parameter list breaks around
    // it; the expression itself must stay intact. Only the real parameter-list
    // trailing comma may appear — never a comma from a fake generic split.
    assert!(
        out.matches(',').count() <= 1,
        "extra comma from a fake generic: {out}"
    );
}

/// Collapse incidental whitespace and drop only a generated trailing comma
/// immediately before a closing angle bracket; real argument separators, token
/// order, and identifier spelling must survive.
fn normalize_type(text: &str) -> String {
    let mut out: String = text.chars().filter(|ch| !ch.is_whitespace()).collect();
    loop {
        let replaced = out.replace(",>", ">");
        if replaced == out {
            return out;
        }
        out = replaced;
    }
}

#[test]
fn genuine_nested_generic_types_keep_token_order_delimiters_and_real_commas() {
    let expected_type =
        "Dictionary<String, Array<Dictionary<String, SomeVeryLongGenericParameterName>>>";
    let source = format!("func make() -> {expected_type} {{ [:] }}\n");
    let out = signatures(&source);

    // Delimiters stay attached to their arguments. Structural line breaks are
    // allowed, but synthesized spaces around `<`/`>` are not.
    assert!(!out.contains(" < "), "{out}");
    assert!(!out.contains(" > "), "{out}");

    let start = out.find("-> ").expect("return arrow") + 3;
    let end = out.rfind(" { ... }").expect("body placeholder");
    let projected_type = normalize_type(&out[start..end]);
    assert_eq!(
        projected_type,
        expected_type
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<String>(),
        "type tokens, order, or names changed: {out}"
    );
    assert_eq!(
        projected_type.matches(',').count(),
        2,
        "the two real argument separators must survive: {out}"
    );
    assert!(
        out.lines().all(|line| line.len() <= 80),
        "nested generic must wrap within the line budget: {out}"
    );
}

// --- R2: actual raw interpolation trivia canonicalizes; literal \( is text --

#[test]
fn one_hash_raw_interpolation_trivia_is_canonicalized() {
    let spaced = r###"func f(_ s: String = #"\#(a   +   b)"#) {}"###;
    let canonical = r###"func f(_ s: String = #"\#(a + b)"#) {}"###;
    let commented = r###"func f(_ s: String = #"\#(a /* note */ + b)"#) {}"###;
    let expected = "func f(_ s: String = #\"\\#(a + b)\"#) { ... }\n";

    assert_eq!(signatures(spaced), expected);
    assert_eq!(signatures(canonical), expected);
    assert_eq!(
        signatures(commented),
        expected,
        "a comment inside an actual interpolation is trivia"
    );
    assert_eq!(signatures(spaced), signatures(canonical));
    assert_eq!(signatures(spaced), signatures(commented));
}

#[test]
fn two_hash_raw_interpolation_trivia_is_canonicalized() {
    let spaced = r###"func f(_ s: String = ##"\##(a   +   b)"##) {}"###;
    let canonical = r###"func f(_ s: String = ##"\##(a + b)"##) {}"###;
    let commented = r###"func f(_ s: String = ##"\##(a /* note */ + b)"##) {}"###;
    let expected = "func f(_ s: String = ##\"\\##(a + b)\"##) { ... }\n";

    assert_eq!(signatures(spaced), expected);
    assert_eq!(signatures(canonical), expected);
    assert_eq!(
        signatures(commented),
        expected,
        "a comment inside an actual interpolation is trivia"
    );
    assert_eq!(signatures(spaced), signatures(canonical));
}

#[test]
fn literal_parenthesis_escape_inside_raw_string_is_not_executable() {
    let source = r###"func f(_ s: String = #"literal \(notExecuted) text"#) {}"###;
    let expected = "func f(_ s: String = #\"literal \\(notExecuted) text\"#) { ... }\n";
    assert_eq!(signatures(source), expected);
    assert!(
        signatures(source).contains(r"\("),
        "the literal \\( must survive byte-exactly"
    );
}

#[test]
fn a_closure_inside_a_raw_interpolation_is_stripped_but_literal_text_stays() {
    let a = r###"func f(_ s: String = #"\#(items.map { print($0) }.joined())"#) {}"###;
    let b = r###"func f(_ s: String = #"\#(items.map { otherBody($0) }.joined())"#) {}"###;
    let expected = "func f(_ s: String = #\"\\#(items.map { ... }.joined())\"#) { ... }\n";
    assert_eq!(signatures(a), expected);
    assert_eq!(
        signatures(a),
        signatures(b),
        "closure body edit must be invisible"
    );
    assert!(!signatures(a).contains("print("), "{}", signatures(a));
    assert!(!signatures(a).contains("otherBody("), "{}", signatures(a));
}

#[test]
fn comment_like_text_inside_a_raw_literal_is_preserved() {
    let source = r###"func f(_ s: String = #"http://example.com /* not a comment */"#) {}"###;
    let expected = "func f(_ s: String = #\"http://example.com /* not a comment */\"#) { ... }\n";
    assert_eq!(signatures(source), expected);
    assert!(signatures(source).contains("/* not a comment */"));
    assert!(signatures(source).contains("http://example.com"));
}
