//! Output and color handling.
//!
//! Projections and diffs are written to stdout, diagnostics to stderr. ANSI
//! styling is applied only here, never in core. Color is the only
//! environment-sensitive behavior: everything else in the document is
//! deterministic (section 10).

use std::fmt::Write as _;
use std::io::{self, Write as _};

use anstream::{AutoStream, ColorChoice as AnstreamChoice};
use anstyle::{AnsiColor, Style};

use crate::args::ColorChoice;
use crate::command::CliError;

/// Which framing the plain document uses, so the styler can classify lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentKind {
    Show,
    Diff,
    Review,
}

/// Writes a rendered document to stdout, styling it only when color is active.
pub fn write_document(kind: DocumentKind, document: &str, color: ColorChoice) -> io::Result<()> {
    if color == ColorChoice::Never {
        // Bypass styling entirely rather than relying on stripping, so
        // `--color=never` is guaranteed byte-for-byte free of escape codes.
        let mut out = std::io::stdout().lock();
        out.write_all(document.as_bytes())?;
        return out.flush();
    }

    let styled = match kind {
        DocumentKind::Show => style_show(document),
        DocumentKind::Diff => style_diff(document),
        DocumentKind::Review => style_review(document),
    };
    // For `auto`, anstream decides from stdout's terminal status and strips the
    // codes when redirected; for `always` it passes them through.
    let mut stream = AutoStream::new(std::io::stdout(), stream_choice(color));
    stream.write_all(styled.as_bytes())?;
    stream.flush()
}

/// Writes plain text to stdout and flushes it.
///
/// Used for pre-flight disclosures, which are never styled, so `--color` cannot
/// change them.
pub fn write_plain(text: &str) -> io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(text.as_bytes())?;
    out.flush()
}

/// Renders a fatal error as a miette report for stderr.
pub fn render_diagnostic(error: &CliError) -> String {
    let mut text = String::new();
    // A fixed width keeps stderr independent of terminal size (section 10);
    // the default theme still detects color support, so redirected stderr
    // stays plain.
    let handler = miette::GraphicalReportHandler::new()
        .with_width(80)
        .with_theme(miette::GraphicalTheme::default());
    if handler.render_report(&mut text, error).is_err() {
        // Writing to a `String` cannot fail; this fallback only avoids a panic
        // if the handler's contract changes.
        let _ = writeln!(text, "{error}");
    }
    text
}

fn stream_choice(color: ColorChoice) -> AnstreamChoice {
    match color {
        ColorChoice::Auto => AnstreamChoice::Auto,
        ColorChoice::Always => AnstreamChoice::Always,
        ColorChoice::Never => AnstreamChoice::Never,
    }
}

/// Bolds the `== path ==` section headers; everything else is left plain.
fn style_show(document: &str) -> String {
    let bold = Style::new().bold();
    let mut styled = String::with_capacity(document.len());
    for line in document.split_inclusive('\n') {
        if is_show_header(line) {
            push_styled(&mut styled, bold, line);
        } else {
            styled.push_str(line);
        }
    }
    styled
}

fn is_show_header(line: &str) -> bool {
    let trimmed = line.trim_end_matches('\n');
    trimmed.starts_with("== ") && trimmed.ends_with(" ==")
}

/// Colors diff lines by kind.
///
/// The `---`/`+++` file headers share their prefixes with deletions and
/// insertions, so a small state machine marks the two lines that structurally
/// follow each `diff --ownai` line as headers instead of content (section 13).
fn style_diff(document: &str) -> String {
    let bold = Style::new().bold();
    let deletion = AnsiColor::Red.on_default();
    let insertion = AnsiColor::Green.on_default();
    let hunk = AnsiColor::Cyan.on_default();

    let mut styled = String::with_capacity(document.len());
    // 0 = hunk body, 2 = the next two lines are the file header pair.
    let mut header_lines_remaining = 0u8;

    for line in document.split_inclusive('\n') {
        if line.starts_with("diff --ownai") {
            push_styled(&mut styled, bold, line);
            header_lines_remaining = 2;
        } else if header_lines_remaining > 0 {
            push_styled(&mut styled, bold, line);
            header_lines_remaining -= 1;
        } else if line.starts_with("@@") {
            push_styled(&mut styled, hunk, line);
        } else if line.starts_with('+') {
            push_styled(&mut styled, insertion, line);
        } else if line.starts_with('-') {
            push_styled(&mut styled, deletion, line);
        } else {
            styled.push_str(line);
        }
    }
    styled
}

/// Styles a review document without misclassifying embedded diff code.
///
/// The header and annotation blocks are OwnAI-authored text; the canonical diff
/// embedded between them must reuse [`style_diff`]'s rules. A state machine
/// tracks whether the current line is inside the diff region, which begins at a
/// `diff --ownai` line and ends at the first blank line. Outside the region only
/// annotation headings contain `" :: "`, so a diff content line such as a
/// Haskell signature is never mistaken for one.
fn style_review(document: &str) -> String {
    let bold = Style::new().bold();
    let deletion = AnsiColor::Red.on_default();
    let insertion = AnsiColor::Green.on_default();
    let hunk = AnsiColor::Cyan.on_default();

    let mut styled = String::with_capacity(document.len());
    // False outside the embedded canonical diff; true from `diff --ownai` until
    // the first blank line.
    let mut in_diff = false;
    // 0 = hunk body, 2 = the next two lines are the file header pair.
    let mut header_lines_remaining = 0u8;

    for line in document.split_inclusive('\n') {
        if line.starts_with("diff --ownai") {
            in_diff = true;
            header_lines_remaining = 2;
            push_styled(&mut styled, bold, line);
            continue;
        }

        if in_diff {
            if line.trim_end_matches('\n').is_empty() {
                // The blank separator ends the diff region.
                in_diff = false;
                styled.push_str(line);
            } else if header_lines_remaining > 0 {
                push_styled(&mut styled, bold, line);
                header_lines_remaining -= 1;
            } else if line.starts_with("@@") {
                push_styled(&mut styled, hunk, line);
            } else if line.starts_with('+') {
                push_styled(&mut styled, insertion, line);
            } else if line.starts_with('-') {
                push_styled(&mut styled, deletion, line);
            } else {
                styled.push_str(line);
            }
            continue;
        }

        if is_review_title(line) || is_review_field(line) || is_review_annotation(line) {
            push_styled(&mut styled, bold, line);
        } else {
            styled.push_str(line);
        }
    }
    styled
}

/// The `ownai review` title line.
fn is_review_title(line: &str) -> bool {
    line.trim_end_matches('\n') == "ownai review"
}

/// A `key: value` header line, matching `^[a-z][a-z-]*: `.
fn is_review_field(line: &str) -> bool {
    let trimmed = line.trim_end_matches('\n');
    let bytes = trimmed.as_bytes();
    if bytes.first().is_none_or(|byte| !byte.is_ascii_lowercase()) {
        return false;
    }
    let mut index = 1;
    while index < bytes.len() && (bytes[index].is_ascii_lowercase() || bytes[index] == b'-') {
        index += 1;
    }
    bytes[index..].starts_with(b": ")
}

/// An annotation heading: `{path-display} :: {stable_key}`.
fn is_review_annotation(line: &str) -> bool {
    line.contains(" :: ")
}

fn push_styled(out: &mut String, style: Style, text: &str) {
    // Writing to a `String` cannot fail, so the result is intentionally ignored
    // rather than unwrapped.
    let _ = write!(out, "{}{}{}", style.render(), text, style.render_reset());
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A review document with an embedded Haskell-style diff, whose content
    /// lines contain `" :: "` and must not be mistaken for annotation headings.
    const REVIEW_DOCUMENT: &str = concat!(
        "ownai review\n",
        "provider: fake\n",
        "units: 1 reviewed, 0 skipped, 0 failed\n",
        "\n",
        "diff --ownai a/src/Model.hs b/src/Model.hs\n",
        "--- a/src/Model.hs\n",
        "+++ b/src/Model.hs\n",
        "@@ -1 +1 @@\n",
        "-foo :: Int -> Int\n",
        "+foo :: Int -> Bool\n",
        "\n",
        "src/Model.hs :: function foo\n",
        "  change: modified\n",
        "  risk: 3.0/5 (confidence 0.70)\n",
    );

    #[test]
    fn review_styling_bolds_header_fields_and_annotation_headings() {
        let styled = style_review(REVIEW_DOCUMENT);

        assert!(styled.contains("\u{1b}[1mownai review\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[1mprovider: fake\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[1munits: 1 reviewed, 0 skipped, 0 failed\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[1msrc/Model.hs :: function foo\n\u{1b}[0m"));
    }

    #[test]
    fn review_styling_colors_the_embedded_diff() {
        let styled = style_review(REVIEW_DOCUMENT);

        assert!(styled.contains("\u{1b}[1mdiff --ownai a/src/Model.hs b/src/Model.hs\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[1m--- a/src/Model.hs\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[1m+++ b/src/Model.hs\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[36m@@ -1 +1 @@\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[31m-foo :: Int -> Int\n\u{1b}[0m"));
        assert!(styled.contains("\u{1b}[32m+foo :: Int -> Bool\n\u{1b}[0m"));
    }

    #[test]
    fn review_styling_does_not_bold_diff_content_with_double_colon() {
        let styled = style_review(REVIEW_DOCUMENT);

        assert!(
            !styled.contains("\u{1b}[1m-foo :: Int -> Int"),
            "a deleted diff line must stay colored, not bold"
        );
        assert!(
            !styled.contains("\u{1b}[1m+foo :: Int -> Bool"),
            "an added diff line must stay colored, not bold"
        );
    }
}
