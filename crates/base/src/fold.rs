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
//! The model is 1-based: [`FoldRegion::start_line`] and [`FoldRegion::end_line`]
//! are 1-based line numbers into the source. [`crate::model::SourceSpan`] is
//! zero-based, so a span's raw lines are its `start_line`/`end_line` plus one.
//!
//! # Bodies
//!
//! [`FoldRegion::end_line`] is the span's end line, which the projection schema
//! guarantees includes the declaration's body. A folded region therefore hides
//! the body; it does not stop at the signature.

use std::collections::{HashMap, HashSet};

use crate::model::{ItemKind, ProjectedItem};
use crate::outline::FileOutline;

/// One declaration's fold region within a file, in source order.
///
/// `start_line` is extended upward over the complete leading annotation block
/// (doc comments, attributes, decorators); `end_line` is the span's end line and
/// includes the body. Regions are returned in outline order, which is source
/// order.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FoldRegion {
    pub stable_key: String,
    pub parent_key: Option<String>,
    pub kind: ItemKind,
    /// Whether the requested mode's projection retains this declaration, taken
    /// from [`FileOutline::retained`] so it cannot disagree with the projection.
    pub retained: bool,
    /// 1-based first line, extended upward over leading annotations.
    pub start_line: usize,
    /// 1-based last line, taken from the span and including the body.
    pub end_line: usize,
    /// 1-based nesting depth found by walking `parent_key` to the top.
    pub level: usize,
    /// Whether the declaration starts unfolded. A retained container is open so
    /// the declarations it holds stay reachable; a retained signature is closed
    /// because the requested mode keeps only its shape; and a declaration the
    /// requested mode drops is closed whatever its kind, so its body is replaced
    /// by a fold row marked `hidden in <mode>`.
    pub default_open: bool,
}

/// Builds fold regions from a file's complete outline and its source lines.
///
/// `outline.outline` is the mode-independent Signatures superset, already in
/// source order, so the result is deterministic and in source order too. `lines`
/// is the buffer's full line list; a line referenced past its end is treated as
/// blank, matching the plugin's `lines[i] or ""`.
///
/// Each region's [`FoldRegion::default_open`] mirrors the plugin's
/// `default_closed`: a declaration the requested mode drops is closed whichever
/// its kind, so the mode's filtering is visible as a fold row rather than a body
/// left open.
///
/// The function is pure: it reads no files and writes nothing.
pub fn fold_regions(outline: &FileOutline, lines: &[&str]) -> Vec<FoldRegion> {
    let items: Vec<&ProjectedItem> = outline.outline.iter().map(|entry| &entry.item).collect();
    let index: HashMap<&str, usize> = items
        .iter()
        .enumerate()
        .map(|(position, item)| (item.stable_key.as_str(), position))
        .collect();
    // Raw 1-based spans, used unchanged for both boundary checks and end lines.
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
                end_line: raw_end,
                level: folded.depth_of(position) + 1,
                default_open: default_open(retained, entry.item.kind),
            }
        })
        .collect()
}

/// Extends a 1-based fold start upward over complete leading annotation blocks.
///
/// A declaration's span excludes its attributes, decorators, and doc comments,
/// so the fold start must walk up over them. `lines` is the buffer's full line
/// list and `is_boundary(line)` answers whether a 1-based line belongs to a
/// different, non-ancestor declaration, which ends a leading run.
///
/// A `trailing` form that directly follows another declaration (a `//`, `#`, or
/// `--` comment owned by the declaration above) stops the run; a forward-
/// attaching form (`///`, `//!`, `/**`, `/*!`, `{-|`, `{-#`, `#[`, `#![`, `@…`)
/// belongs to the declaration below and may cross into the preceding
/// declaration's span.
///
/// A `start_line` of zero or one has nothing above it to claim and is returned
/// unchanged.
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

/// Whether a declaration starts open (its body visible) or closed (folded).
///
/// A declaration the requested mode drops is closed whatever its kind, so its
/// body is replaced by a `hidden in <mode>` fold row; a retained container is
/// open so the declarations it holds stay reachable; and a retained signature is
/// closed because the requested mode already keeps only its shape. This mirrors
/// the plugin's `default_closed` (`editors/nvim/lua/codect/state.lua`).
fn default_open(retained: bool, kind: ItemKind) -> bool {
    retained && is_open_kind(kind)
}

/// Whether a retained declaration of this kind starts unfolded.
///
/// Containers are open so the declarations they hold stay reachable; signatures
/// are closed. The match is exhaustive with no wildcard, so a new [`ItemKind`]
/// is a compile error here rather than a silent default.
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

/// The derived view of one file's items that boundary and nesting queries need.
struct FoldInput<'a> {
    items: Vec<&'a ProjectedItem>,
    index: HashMap<&'a str, usize>,
    /// Raw 1-based inclusive spans, parallel to `items`.
    spans: Vec<(usize, usize)>,
}

impl FoldInput<'_> {
    /// True when `items[j]` is an ancestor of `items[i]` (following `parent_key`
    /// up from `i`). A cycle is cut off rather than followed forever.
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

    /// True when 1-based `line` belongs to a different, non-ancestor
    /// declaration: the boundary a leading run must not cross.
    fn is_boundary(&self, i: usize, line: usize) -> bool {
        self.spans.iter().enumerate().any(|(j, &(start, end))| {
            j != i && (start..=end).contains(&line) && !self.is_ancestor(i, j)
        })
    }

    /// The number of ancestors of `items[i]`. A missing parent ends the chain; a
    /// cycle (which the outline constructor forbids) is treated as depth zero.
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

/// A block annotation's opener and closer.
#[derive(Clone, Copy)]
struct BlockSpec {
    open: &'static str,
    close: &'static str,
}

/// Block annotations, closest-first: `{-#` must precede `{-`, and `#![` must
/// precede `#[`, because each former's closer also matches the latter.
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

/// How a leading annotation block attaches to the declaration below it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum AnnotationForm {
    /// Belongs to the declaration below and may cross into the one above.
    Forward,
    /// Belongs to the declaration above when it directly follows it.
    Trailing,
}

/// The start line of a complete leading annotation block ending at `last`, plus
/// its form, or `None` when `last` does not end one. `last` is 1-based.
fn annotation_start(lines: &[&str], last: usize) -> Option<(usize, Option<AnnotationForm>)> {
    let raw = line(lines, last);
    if is_blank(raw) {
        return None;
    }
    let trimmed = ltrim(raw);

    // A shebang on line 1 is a file directive, not a declaration annotation.
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

/// A single-line comment. `#[` and `#![` open Rust attributes, not comments,
/// and `#-}` closes a Haskell `{-# … #-}` pragma.
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

/// Classifies an annotation block by its opening line.
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

/// The line that opens a block annotation ending at `last`, or `None`.
///
/// Scans upward tracking nesting depth (string-aware), so a balanced inner block
/// (`/* inner */`) is skipped and the outermost opener is returned. Blank lines
/// inside the already-open block are tolerated; a blank line between separate
/// annotation runs still ends the search.
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
        // depth > 0: inside the block. A blank line here is interior.
    }
    None
}

/// The line that opens a `@decorator` ending at `last`, including parenthesized
/// or continued forms, or `None`.
///
/// Blank lines inside the parentheses are tolerated; the search still stops at a
/// blank or uncontinued line between separate runs.
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

/// Replaces the contents of string literals with spaces.
///
/// Delimiter counting must ignore brackets inside strings: `#[doc = "a ] b"]`
/// has one `[` and one `]`, not one and two. Raw/byte-string prefixes are
/// blanked too; they are never delimiters. A Rust lifetime (`'a`, `'static`) is
/// copied verbatim rather than treated as a char literal.
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

/// The index just past a quoted string starting at `start`.
///
/// Handles `\` escapes and an unterminated string (consumes the rest of the
/// line). `'` is treated as a string delimiter, which is right for Python
/// decorators and Rust char literals; a bare lifetime is handled by
/// [`lifetime_end`] before this is reached.
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

/// The index just past a raw/byte string starting at `start`, or `None`.
///
/// Recognizes `r"…"`, `r#"…"#`, `br"…"`, `br#"…"#`, and `rb…` forms. The closing
/// quote must be followed by the same number of `#` characters.
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

/// The index just past a Rust lifetime starting at `start` (`'a`, `'static`), or
/// `None` when the `'` opens a char literal. A lifetime has no closing quote.
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

/// The index just past the first occurrence of `needle` at or after `from`, or
/// `haystack.len()` when there is none (an unterminated literal consumes its
/// line).
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

/// The 1-based `number` line, or `""` when it is past the end. Matches the
/// plugin's `lines[i] or ""`.
fn line<'a>(lines: &[&'a str], number: usize) -> &'a str {
    number
        .checked_sub(1)
        .and_then(|position| lines.get(position))
        .copied()
        .unwrap_or("")
}

/// True for an empty line or one made only of Lua `%s` whitespace, matching the
/// plugin's byte-oriented blank test.
fn is_blank(s: &str) -> bool {
    s.as_bytes().iter().all(|byte| is_lua_space(*byte))
}

/// Lua's `%s` set, byte for byte, so blank and trim tests agree with the plugin.
fn is_lua_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | 0x0B | 0x0C | b'\r')
}

/// Removes leading Lua whitespace. ASCII spaces are char boundaries, so the
/// result is always a valid slice.
fn ltrim(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut start = 0;
    while start < bytes.len() && is_lua_space(bytes[start]) {
        start += 1;
    }
    &s[start..]
}

/// Whether `haystack` contains `needle` as a byte substring.
fn contains_bytes(haystack: &[u8], needle: &str) -> bool {
    let needle = needle.as_bytes();
    !needle.is_empty()
        && haystack
            .windows(needle.len())
            .any(|window| window == needle)
}

/// Counts non-overlapping occurrences of `needle`, matching `count_plain`.
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

    /// A `ProjectedItem` with zero-based `start_line`/`end_line`. Byte offsets
    /// and columns are irrelevant to folding.
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

    /// All `superset` items are retained by passing them as the projection too.
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
            // Zero-based span lines 2..=5 -> 1-based 3..=6.
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
            // Zero-based span lines 2..=4 -> 1-based 3..=5, doc comment on line 2.
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
        // An inherent `impl` is a container, but the mode drops it: it must fold
        // (not stay open and render its body as source), matching the plugin.
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
        // The mode's filtering wins over the kind: a dropped container folds just
        // as a dropped signature does, matching the plugin's `default_closed`.
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

        // With line 1 known to be another declaration's, the trailing comment is
        // not claimed.
        assert_eq!(fold_start(&lines, 3, |line| line == 1), 3);

        // Without that boundary the trailing comment is claimed, because a `//`
        // comment on its own is a valid leading annotation.
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
        // A missing line cannot be an annotation, so the start is unchanged.
        assert_eq!(fold_start(&lines, 5, |_| false), 5);
        assert_eq!(annotation_start(&lines, 5), None);
    }
}
