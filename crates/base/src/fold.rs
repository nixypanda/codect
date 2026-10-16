//! The source-buffer fold model.
//!
//! This is the grammar-agnostic half of the Neovim plugin's fold logic
//! (`editors/nvim/lua/codect/outline.lua`): given a file's complete outline and
//! the decoded source lines, it reports one fold region per declaration. It
//! knows only the model types and the source text, so `base` stays free of
//! grammars, node names, and file formats.
//!
//! # Lines
//!
//! The model is 1-based: [`FoldRegion::start_line`],
//! [`FoldRegion::declaration_line`], and [`FoldRegion::end_line`] are 1-based
//! line numbers into the source. [`crate::model::SourceSpan`] is zero-based, so
//! a span's raw lines are its `start_line`/`end_line` plus one.
//!
//! Two lines describe a region's start. `start_line` is the extended fold start:
//! it walks upward over the declaration's complete leading annotation block
//! (doc comments, attributes, decorators), so collapsing from there hides the
//! annotations too. `declaration_line` is the declaration node's own raw start,
//! the first line that is not a leading annotation; it is the line a collapsed
//! row summarizes, so the row shows the declaration's signature rather than the
//! annotation above it. The two are equal when the declaration has no leading
//! annotation.
//!
//! # Bodies
//!
//! [`FoldRegion::end_line`] is the span's end line, which the projection schema
//! guarantees includes the declaration's body. A folded region therefore hides
//! the body; it does not stop at the signature.
//!
//! # Modes
//!
//! A declaration the requested mode drops is closed whatever its kind, so its
//! body becomes a `hidden in <mode>` fold row; a retained container is open so
//! its declarations stay reachable, and a retained signature is closed.
//!
//! Block annotation openers are tried closest-first (`{-#` before `{-`, `#![`
//! before `#[`) because the shorter opener's closer also matches the longer
//! opener's.

use std::collections::{HashMap, HashSet};

use crate::model::{ItemKind, ProjectedItem};
use crate::outline::FileOutline;

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldRegion {
    pub stable_key: String,
    pub parent_key: Option<String>,
    pub kind: ItemKind,
    pub retained: bool,
    pub start_line: usize,
    pub declaration_line: usize,
    pub end_line: usize,
    pub level: usize,
    pub default_open: bool,
}

pub fn fold_regions(outline: &FileOutline, lines: &[&str]) -> Vec<FoldRegion> {
    let items: Vec<&ProjectedItem> = outline.outline.iter().map(|entry| &entry.item).collect();
    let index: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .map(|(position, item)| (item.stable_key.as_str(), position))
        .collect();
    let spans: Vec<(usize, usize)> = items
        .iter()
        .map(|item| (item.span.start_line() + 1, item.span.end_line() + 1))
        .collect();
    let folded = FoldInput {
        items,
        index,
        spans,
    };

    outline
        .outline
        .iter()
        .enumerate()
        .map(|(position, entry)| {
            let (raw_start, raw_end) = folded.spans[position];
            let retained = outline.retained_in_mode(entry);
            FoldRegion {
                stable_key: entry.item.stable_key.clone(),
                parent_key: entry.item.parent_key.clone(),
                kind: entry.item.kind,
                retained,
                start_line: fold_start(lines, raw_start, |line| folded.is_boundary(position, line)),
                declaration_line: raw_start,
                end_line: raw_end,
                level: folded.depth_of(position) + 1,
                default_open: default_open(retained, entry.item.kind),
            }
        })
        .collect()
}

pub fn fold_start(lines: &[&str], start_line: usize, is_boundary: impl Fn(usize) -> bool) -> usize {
    let mut line = start_line;
    while line > 1 {
        let Some((block_start, form)) = annotation_start(lines, line - 1) else {
            break;
        };
        if form == Some(AnnotationForm::Trailing) && block_start > 1 && is_boundary(block_start - 1)
        {
            break;
        }
        line = block_start;
    }
    line
}

fn default_open(retained: bool, kind: ItemKind) -> bool {
    retained && is_open_kind(kind)
}

fn is_open_kind(kind: ItemKind) -> bool {
    match kind {
        ItemKind::Module
        | ItemKind::Type
        | ItemKind::TypeAlias
        | ItemKind::Constructor
        | ItemKind::Field
        | ItemKind::Variant
        | ItemKind::Trait
        | ItemKind::TraitImplementation
        | ItemKind::AssociatedType
        | ItemKind::TypeFamily
        | ItemKind::PatternSynonym => true,
        ItemKind::Function
        | ItemKind::Method
        | ItemKind::Value
        | ItemKind::Constant
        | ItemKind::Static
        | ItemKind::Port
        | ItemKind::Operator
        | ItemKind::ForeignBlock => false,
    }
}

struct FoldInput<'a> {
    items: Vec<&'a ProjectedItem>,
    index: HashMap<&'a str, usize>,
    spans: Vec<(usize, usize)>,
}

impl FoldInput<'_> {
    fn is_ancestor(&self, i: usize, j: usize) -> bool {
        let key = self.items[j].stable_key.as_str();
        let mut current = self.items[i].parent_key.as_deref();
        let mut walked = 0;
        while let Some(parent) = current {
            if parent == key {
                return true;
            }
            walked += 1;
            if walked > self.items.len() {
                break;
            }
            current = self
                .index
                .get(parent)
                .and_then(|&position| self.items[position].parent_key.as_deref());
        }
        false
    }

    fn is_boundary(&self, i: usize, line: usize) -> bool {
        self.spans.iter().enumerate().any(|(j, &(start, end))| {
            j != i && (start..=end).contains(&line) && !self.is_ancestor(i, j)
        })
    }

    fn depth_of(&self, i: usize) -> usize {
        let mut depth = 0;
        let mut seen = HashSet::new();
        seen.insert(i);
        let mut current = self.items[i].parent_key.as_deref();
        while let Some(parent) = current {
            let Some(&position) = self.index.get(parent) else {
                break;
            };
            if !seen.insert(position) {
                return 0;
            }
            depth += 1;
            current = self.items[position].parent_key.as_deref();
        }
        depth
    }
}

#[derive(Clone, Copy)]
struct BlockSpec {
    open: &'static str,
    close: &'static str,
}

const BLOCK_SPECS: [BlockSpec; 5] = [
    BlockSpec {
        open: "/*",
        close: "*/",
    },
    BlockSpec {
        open: "{-#",
        close: "#-}",
    },
    BlockSpec {
        open: "{-",
        close: "-}",
    },
    BlockSpec {
        open: "#![",
        close: "]",
    },
    BlockSpec {
        open: "#[",
        close: "]",
    },
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnnotationForm {
    Forward,
    Trailing,
}

fn annotation_start(lines: &[&str], last: usize) -> Option<(usize, Option<AnnotationForm>)> {
    let raw = line(lines, last);
    if is_blank(raw) {
        return None;
    }
    let trimmed = ltrim(raw);

    if last == 1 && trimmed.starts_with("#!") && !trimmed.starts_with("#![") {
        return None;
    }

    if is_line_comment(trimmed) {
        return Some((last, annotation_form(trimmed)));
    }

    if let Some(decorator) = find_decorator_start(lines, last) {
        return Some((decorator, annotation_form(ltrim(line(lines, decorator)))));
    }

    let scanned = strip_strings(raw);
    for spec in BLOCK_SPECS {
        if contains_bytes(&scanned, spec.close)
            && let Some(start) = find_block_start(lines, last, spec)
        {
            return Some((start, annotation_form(ltrim(line(lines, start)))));
        }
    }

    None
}

fn is_line_comment(s: &str) -> bool {
    if s.starts_with("///") || s.starts_with("//!") || s.starts_with("//") {
        return true;
    }
    if s.starts_with("--") {
        return true;
    }
    if s.starts_with('#') {
        if s.starts_with("#[") || s.starts_with("#![") || s.starts_with("#-}") {
            return false;
        }
        return true;
    }
    false
}

fn annotation_form(s: &str) -> Option<AnnotationForm> {
    if s.starts_with("///") || s.starts_with("//!") {
        return Some(AnnotationForm::Forward);
    }
    if s.starts_with("//") {
        return Some(AnnotationForm::Trailing);
    }
    if s.starts_with("/**") || s.starts_with("/*!") {
        return Some(AnnotationForm::Forward);
    }
    if s.starts_with("/*") {
        return Some(AnnotationForm::Trailing);
    }
    if s.starts_with("{-|") || s.starts_with("{-#") {
        return Some(AnnotationForm::Forward);
    }
    if s.starts_with("{-") {
        return Some(AnnotationForm::Trailing);
    }
    if s.starts_with("#[") || s.starts_with("#![") {
        return Some(AnnotationForm::Forward);
    }
    if s.starts_with('@') {
        return Some(AnnotationForm::Forward);
    }
    if s.starts_with("--") || s.starts_with('#') {
        return Some(AnnotationForm::Trailing);
    }
    None
}

fn find_block_start(lines: &[&str], last: usize, spec: BlockSpec) -> Option<usize> {
    let mut depth: i64 = 0;
    for number in (1..=last).rev() {
        let raw = line(lines, number);
        let scanned = strip_strings(raw);
        let opens = count_plain(&scanned, spec.open) as i64;
        let closes = count_plain(&scanned, spec.close) as i64;
        if number == last {
            depth = closes - opens;
        } else {
            depth += closes - opens;
        }

        if depth <= 0 {
            if depth == 0 && ltrim(raw).starts_with(spec.open) {
                return Some(number);
            }
            return None;
        }
    }
    None
}

fn find_decorator_start(lines: &[&str], last: usize) -> Option<usize> {
    let mut depth: i64 = 0;
    for number in (1..=last).rev() {
        let raw = line(lines, number);
        let scanned = strip_strings(raw);
        depth += count_plain(&scanned, ")") as i64;
        depth -= count_plain(&scanned, "(") as i64;

        if depth <= 0 {
            if depth == 0 && ltrim(raw).starts_with('@') {
                return Some(number);
            }
            return None;
        }
    }
    None
}

fn strip_strings(source: &str) -> Vec<u8> {
    let bytes = source.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let mut stop = raw_string_end(bytes, i);
        let mut handled = false;
        if stop.is_none() {
            match bytes[i] {
                b'\'' => {
                    if let Some(lifetime) = lifetime_end(bytes, i) {
                        out.extend_from_slice(&bytes[i..lifetime]);
                        i = lifetime;
                        handled = true;
                    } else {
                        stop = Some(quoted_string_end(bytes, i));
                    }
                }
                b'"' => stop = Some(quoted_string_end(bytes, i)),
                _ => {}
            }
        }
        if !handled {
            match stop {
                Some(end) => {
                    out.resize(out.len() + (end - i), b' ');
                    i = end;
                }
                None => {
                    out.push(bytes[i]);
                    i += 1;
                }
            }
        }
    }
    out
}

fn quoted_string_end(bytes: &[u8], start: usize) -> usize {
    let quote = bytes[start];
    let mut i = start + 1;
    while i < bytes.len() {
        let byte = bytes[i];
        if byte == b'\\' {
            i += 2;
        } else if byte == quote {
            return i + 1;
        } else {
            i += 1;
        }
    }
    bytes.len()
}

fn raw_string_end(bytes: &[u8], start: usize) -> Option<usize> {
    let rest = &bytes[start..];
    for prefix in [b"br".as_slice(), b"rb".as_slice(), b"r".as_slice()] {
        let Some(after_prefix) = rest.strip_prefix(prefix) else {
            continue;
        };
        let hashes = after_prefix
            .iter()
            .take_while(|&&byte| byte == b'#')
            .count();
        let Some(&quote) = after_prefix.get(hashes) else {
            continue;
        };
        if quote != b'"' && quote != b'\'' {
            continue;
        }
        let mut closer = vec![quote];
        closer.extend(std::iter::repeat_n(b'#', hashes));
        let content_start = start + prefix.len() + hashes + 1;
        return Some(find_bytes(bytes, content_start, &closer));
    }
    None
}

fn lifetime_end(bytes: &[u8], start: usize) -> Option<usize> {
    let mut i = start + 1;
    if !matches!(bytes.get(i), Some(byte) if byte.is_ascii_alphabetic() || *byte == b'_') {
        return None;
    }
    i += 1;
    while matches!(bytes.get(i), Some(byte) if byte.is_ascii_alphanumeric() || *byte == b'_') {
        i += 1;
    }
    if bytes.get(i) == Some(&b'\'') {
        None
    } else {
        Some(i)
    }
}

fn find_bytes(haystack: &[u8], from: usize, needle: &[u8]) -> usize {
    if needle.is_empty() || from > haystack.len() {
        return haystack.len();
    }
    match haystack[from..]
        .windows(needle.len())
        .position(|window| window == needle)
    {
        Some(offset) => from + offset + needle.len(),
        None => haystack.len(),
    }
}

fn line<'a>(lines: &[&'a str], number: usize) -> &'a str {
    number
        .checked_sub(1)
        .and_then(|position| lines.get(position))
        .copied()
        .unwrap_or("")
}

fn is_blank(s: &str) -> bool {
    s.as_bytes().iter().all(|byte| is_lua_space(*byte))
}

fn is_lua_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

fn ltrim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_lua_space(bytes[start]) {
        start += 1;
    }
    &s[start..]
}

fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

fn count_plain(haystack: &[u8], needle: &str) -> usize {
    let needle = needle.as_bytes();
    if needle.is_empty() {
        return 0;
    }
    let mut count = 0;
    let mut i = 0;
    while i + needle.len() <= haystack.len() {
        if &haystack[i..i + needle.len()] == needle {
            count += 1;
            i += needle.len();
        } else {
            i += 1;
        }
    }
    count
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ProjectedItem, RepoPath, SourceSpan, SupportedPath};
    use crate::outline::{FileOutline, assemble_outline};

    fn supported(raw: &str) -> SupportedPath {
        SupportedPath::new(RepoPath::new(raw).unwrap()).unwrap()
    }

    fn projected(
        key: &str,
        parent: Option<&str>,
        kind: ItemKind,
        start_line: usize,
        end_line: usize,
    ) -> ProjectedItem {
        ProjectedItem {
            stable_key: key.to_owned(),
            parent_key: parent.map(str::to_owned),
            kind,
            name: key.to_owned(),
            span: SourceSpan::new(0, 0, start_line, 0, end_line, 0),
            canonical_text: format!("{key};"),
        }
    }

    fn outline_all(superset: Vec<ProjectedItem>) -> FileOutline {
        assemble_outline(&supported("src/lib.rs"), superset.clone(), superset)
            .expect("valid outline fixture")
    }

    #[test]
    fn a_leading_doc_comment_folds_into_a_retained_function_and_the_body_is_included() {
        let lines = [
            "/// Doc line one.",
            "/// Doc line two.",
            "pub fn documented() -> u32 {",
            "    let x = 1;",
            "    x",
            "}",
            "",
            "pub fn other() {}",
        ];
        let file = outline_all(vec![
            projected("fn::documented", None, ItemKind::Function, 2, 5),
            projected("fn::other", None, ItemKind::Function, 7, 7),
        ]);

        let regions = fold_regions(&file, &lines);

        assert_eq!(regions.len(), 2);
        assert_eq!(regions[0].start_line, 1, "the doc comment is folded in");
        assert_eq!(regions[0].end_line, 6, "the body is inside the region");
        assert!(regions[0].retained);
        assert!(!regions[0].default_open, "a function is folded by default");
        assert_eq!(regions[1].start_line, 8);
        assert_eq!(regions[1].end_line, 8);
    }

    #[test]
    fn an_attribute_line_extends_the_fold_start_but_not_the_declaration_line() {
        let lines = ["#[test]", "fn tested() {", "    assert!(true);", "}"];
        let file = outline_all(vec![projected(
            "fn::tested",
            None,
            ItemKind::Function,
            1,
            3,
        )]);

        let region = &fold_regions(&file, &lines)[0];
        assert_eq!(region.start_line, 1, "the attribute is folded in");
        assert_eq!(
            region.declaration_line, 2,
            "the declaration itself starts on the `fn` line"
        );
        assert_eq!(region.end_line, 4, "the body is inside the region");
    }

    #[test]
    fn a_declaration_without_annotations_has_equal_start_and_declaration_lines() {
        let lines = ["fn plain() {", "    1", "}"];
        let file = outline_all(vec![projected("fn::plain", None, ItemKind::Function, 0, 2)]);

        let region = &fold_regions(&file, &lines)[0];
        assert_eq!(region.start_line, 1);
        assert_eq!(
            region.declaration_line, 1,
            "without a leading annotation the two lines coincide"
        );
    }

    #[test]
    fn a_doc_comment_lead_in_points_the_declaration_line_at_the_function() {
        let lines = [
            "/// Doc line one.",
            "/// Doc line two.",
            "pub fn documented() -> u32 {",
            "    1",
            "}",
        ];
        let file = outline_all(vec![projected(
            "fn::documented",
            None,
            ItemKind::Function,
            2,
            4,
        )]);

        let region = &fold_regions(&file, &lines)[0];
        assert_eq!(region.start_line, 1, "the doc comment is folded in");
        assert_eq!(
            region.declaration_line, 3,
            "the declaration itself starts on the `fn` line"
        );
    }

    #[test]
    fn a_trailing_line_comment_after_another_declaration_is_not_claimed() {
        let rust = [
            "pub fn first() -> u32 { 5 }",
            "// trailing comment that belongs to first",
            "pub fn second() -> u32 { 6 }",
        ];
        let python = [
            "def first():",
            "    return 1",
            "# trailing comment that belongs to first",
            "def second():",
            "    return 2",
        ];
        let haskell = [
            "foo :: Int -> Int",
            "foo x = x",
            "-- trailing comment that belongs to foo",
            "bar :: Int -> Int",
        ];

        let rust_outline = outline_all(vec![
            projected("fn::first", None, ItemKind::Function, 0, 0),
            projected("fn::second", None, ItemKind::Function, 2, 2),
        ]);
        let python_outline = outline_all(vec![
            projected("fn::first", None, ItemKind::Function, 0, 1),
            projected("fn::second", None, ItemKind::Function, 3, 4),
        ]);
        let haskell_outline = outline_all(vec![
            projected("foo", None, ItemKind::Value, 0, 1),
            projected("bar", None, ItemKind::Type, 3, 3),
        ]);

        assert_eq!(fold_regions(&rust_outline, &rust)[1].start_line, 3);
        assert_eq!(fold_regions(&python_outline, &python)[1].start_line, 4);
        assert_eq!(fold_regions(&haskell_outline, &haskell)[1].start_line, 4);
    }

    #[test]
    fn a_forward_doc_comment_crosses_the_preceding_declaration() {
        let lines = ["pub fn a() {}", "/// doc for c", "pub fn c() {}"];
        let file = outline_all(vec![
            projected("fn::a", None, ItemKind::Function, 0, 0),
            projected("fn::c", None, ItemKind::Function, 2, 2),
        ]);

        let regions = fold_regions(&file, &lines);
        assert_eq!(
            regions[1].start_line, 2,
            "`///` attaches to the next method"
        );
    }

    #[test]
    fn block_comments_attributes_and_decorators_attach_forward() {
        let rust = [
            "/* plain block comment */",
            "pub const PLAIN: u32 = 3;",
            "/*! inner block comment */",
            "pub const INNER: u32 = 4;",
        ];
        let rust_outline = outline_all(vec![
            projected("PLAIN", None, ItemKind::Constant, 1, 1),
            projected("INNER", None, ItemKind::Constant, 3, 3),
        ]);
        let regions = fold_regions(&rust_outline, &rust);
        assert_eq!(regions[0].start_line, 1, "`/* */` folds in");
        assert_eq!(regions[1].start_line, 3, "`/*! */` folds in");

        let elm = ["{- a plain", "   block comment -}", "f x =", "    x"];
        let elm_outline = outline_all(vec![projected("f", None, ItemKind::Value, 2, 3)]);
        assert_eq!(fold_regions(&elm_outline, &elm)[0].start_line, 1);

        let haskell = [
            "{-# LANGUAGE",
            "    MultiParamTypeClasses",
            "    #-}",
            "module Annotated where",
        ];
        let haskell_outline = outline_all(vec![projected(
            "module::Annotated",
            None,
            ItemKind::Module,
            3,
            3,
        )]);
        assert_eq!(fold_regions(&haskell_outline, &haskell)[0].start_line, 1);

        let attributes = [
            "#![allow(dead_code)]",
            "#[cfg(",
            "    feature = \"serde\"",
            ")]",
            "pub fn multi_attr() -> u32 { 2 }",
        ];
        let attributes_outline = outline_all(vec![projected(
            "fn::multi_attr",
            None,
            ItemKind::Function,
            4,
            4,
        )]);
        assert_eq!(
            fold_regions(&attributes_outline, &attributes)[0].start_line,
            1,
            "`#![…]` and a multi-line `#[…]` fold in"
        );
    }

    #[test]
    fn a_python_decorator_including_a_parenthesized_continuation_attaches_forward() {
        let simple = ["@staticmethod", "def simple():", "    return 1"];
        let simple_outline = outline_all(vec![projected(
            "def::simple",
            None,
            ItemKind::Function,
            1,
            2,
        )]);
        assert_eq!(fold_regions(&simple_outline, &simple)[0].start_line, 1);

        let continued = [
            "@app.route(",
            "    \"/health\",",
            "",
            "    methods=[\"GET\"],",
            ")",
            "def health():",
            "    return {}",
        ];
        let continued_outline = outline_all(vec![projected(
            "def::health",
            None,
            ItemKind::Function,
            5,
            6,
        )]);
        assert_eq!(
            fold_regions(&continued_outline, &continued)[0].start_line,
            1,
            "a blank line inside a continued decorator is not a separator"
        );
    }

    #[test]
    fn a_shebang_on_line_one_is_never_claimed() {
        let lines = ["#!/usr/bin/env python", "def first():", "    return 1"];
        let file = outline_all(vec![projected(
            "def::first",
            None,
            ItemKind::Function,
            1,
            2,
        )]);

        assert_eq!(fold_regions(&file, &lines)[0].start_line, 2);
    }

    #[test]
    fn brackets_inside_strings_raw_strings_and_char_literals_do_not_unbalance_attributes() {
        let lines = [
            "#[doc = \"a ] b\"]",
            "pub fn bracket_in_string() -> u32 { 1 }",
            "",
            "#[doc = \"a [ b\"]",
            "pub fn open_bracket_in_string() -> u32 { 1 }",
            "",
            "#[doc = r\"a ] b\"]",
            "pub fn raw_string_attr() -> u32 { 1 }",
            "",
            "#[doc = r#\"a ] b\"#]",
            "pub fn raw_hash_attr() -> u32 { 1 }",
            "",
            "#[doc = ']']",
            "pub fn char_literal_attr() -> u32 { 1 }",
            "",
            "#[foo(bar = 'a)]",
            "pub fn lifetime_attr() -> u32 { 1 }",
        ];
        let file = outline_all(
            [1usize, 4, 7, 10, 13, 16]
                .into_iter()
                .enumerate()
                .map(|(position, start)| {
                    projected(
                        &format!("fn::{position}"),
                        None,
                        ItemKind::Function,
                        start,
                        start,
                    )
                })
                .collect(),
        );

        let regions = fold_regions(&file, &lines);
        for (position, region) in regions.iter().enumerate() {
            let expected = [1usize, 4, 7, 10, 13, 16][position];
            assert_eq!(
                region.start_line, expected,
                "attribute folds in for item {position}"
            );
        }
    }

    #[test]
    fn nesting_follows_parent_key_and_start_lines_do_not_cross_a_trailing_comment_boundary() {
        let lines = [
            "impl Widget {",
            "    /// Build a widget.",
            "    pub fn new() -> Self {",
            "        Self {}",
            "    }",
            "",
            "    pub fn size(&self) -> usize {",
            "        0",
            "    }",
            "}",
        ];
        let file = outline_all(vec![
            projected("impl Widget", None, ItemKind::TraitImplementation, 0, 9),
            projected(
                "impl Widget::new",
                Some("impl Widget"),
                ItemKind::Method,
                2,
                4,
            ),
            projected(
                "impl Widget::size",
                Some("impl Widget"),
                ItemKind::Method,
                6,
                8,
            ),
        ]);

        let regions = fold_regions(&file, &lines);

        assert_eq!(regions.len(), 3);
        assert_eq!(regions[0].stable_key, "impl Widget");
        assert_eq!(regions[0].parent_key, None);
        assert_eq!(regions[0].level, 1);
        assert!(regions[0].default_open, "an impl is open by default");
        assert_eq!(regions[0].start_line, 1);
        assert_eq!(regions[0].end_line, 10);

        assert_eq!(regions[1].parent_key.as_deref(), Some("impl Widget"));
        assert_eq!(regions[1].level, 2, "a method nests inside the impl");
        assert!(!regions[1].default_open, "a method is folded by default");
        assert_eq!(
            regions[1].start_line, 2,
            "the method's doc comment folds in"
        );
        assert_eq!(regions[1].end_line, 5);

        assert_eq!(regions[2].parent_key.as_deref(), Some("impl Widget"));
        assert_eq!(regions[2].level, 2);
        assert_eq!(regions[2].start_line, 7);
        assert_eq!(regions[2].end_line, 9);
        assert_ne!(
            regions[1].start_line, regions[2].start_line,
            "siblings get distinct regions"
        );
    }

    #[test]
    fn retention_comes_from_the_requested_mode_projection() {
        let lines = ["pub fn dropped() {}"];
        let superset = vec![projected("fn::dropped", None, ItemKind::Function, 0, 0)];
        let file = assemble_outline(&supported("src/lib.rs"), Vec::new(), superset)
            .expect("valid fixture");

        let regions = fold_regions(&file, &lines);
        assert_eq!(
            regions.len(),
            1,
            "the outline still locates the dropped item"
        );
        assert!(!regions[0].retained);
        assert!(
            !regions[0].default_open,
            "a dropped signature is closed by default"
        );
    }

    #[test]
    fn a_dropped_container_is_closed_so_the_mode_actually_filters_it() {
        let lines = ["impl Widget {", "    fn size(&self) -> usize { 0 }", "}"];
        let region = projected("impl Widget", None, ItemKind::TraitImplementation, 0, 2);
        let file = assemble_outline(&supported("src/lib.rs"), Vec::new(), vec![region.clone()])
            .expect("valid fixture");

        let regions = fold_regions(&file, &lines);
        assert_eq!(regions.len(), 1);
        assert!(!regions[0].retained, "the impl is dropped by the mode");
        assert!(
            !regions[0].default_open,
            "a dropped container folds so its body is hidden"
        );
    }

    #[test]
    fn default_open_follows_the_signature_kind_rule() {
        let folded = [
            ItemKind::Function,
            ItemKind::Method,
            ItemKind::Value,
            ItemKind::Constant,
            ItemKind::Static,
            ItemKind::Port,
            ItemKind::Operator,
            ItemKind::ForeignBlock,
        ];
        let open = [
            ItemKind::Module,
            ItemKind::Type,
            ItemKind::TypeAlias,
            ItemKind::Constructor,
            ItemKind::Field,
            ItemKind::Variant,
            ItemKind::Trait,
            ItemKind::TraitImplementation,
            ItemKind::AssociatedType,
            ItemKind::TypeFamily,
            ItemKind::PatternSynonym,
        ];

        for kind in folded {
            assert!(!is_open_kind(kind), "{kind:?} must be folded by default");
        }
        for kind in open {
            assert!(is_open_kind(kind), "{kind:?} must be open by default");
        }
    }

    #[test]
    fn default_open_closes_everything_the_mode_drops_of_any_kind() {
        assert!(
            !default_open(false, ItemKind::TraitImplementation),
            "a dropped container must be closed"
        );
        assert!(
            !default_open(false, ItemKind::Function),
            "a dropped signature must be closed"
        );
        assert!(
            default_open(true, ItemKind::TraitImplementation),
            "a retained container is open"
        );
        assert!(
            !default_open(true, ItemKind::Function),
            "a retained signature is closed"
        );
    }

    #[test]
    fn fold_start_stops_at_a_trailing_comment_owned_by_the_previous_declaration() {
        let lines = ["pub fn a() {}", "// trailing", "pub fn b() {}"];

        assert_eq!(fold_start(&lines, 3, |line| line == 1), 3);

        assert_eq!(fold_start(&lines, 3, |_| false), 2);
    }

    #[test]
    fn fold_start_is_unchanged_at_the_top_of_the_file() {
        let lines = ["pub fn a() {}"];
        assert_eq!(fold_start(&lines, 1, |_| true), 1);
    }

    #[test]
    fn lines_past_the_end_are_treated_as_blank() {
        let lines = ["pub fn a() {}"];
        assert_eq!(fold_start(&lines, 5, |_| false), 5);
        assert_eq!(annotation_start(&lines, 5), None);
    }
}
