// Output and color handling.
//
// Projections and diffs are written to stdout, diagnostics to stderr. ANSI
// styling is applied only here, never in core. Color is the only
// environment-sensitive behavior: everything else in the document is
// deterministic (section 10).

use std::fmt::Write as _;
use std::io::{self, Write as _};

use anstream::{AutoStream, ColorChoice as AnstreamChoice};
use anstyle::{AnsiColor, Style};

use crate::args::ColorChoice;
use crate::command::CliError;

// Which framing the plain document uses, so the styler can classify lines.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DocumentKind {
    Show,
    Diff,
}

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
    };
    // For `auto`, anstream decides from stdout's terminal status and strips the
    // codes when redirected; for `always` it passes them through.
    let mut stream = AutoStream::new(std::io::stdout(), stream_choice(color));
    stream.write_all(styled.as_bytes())?;
    stream.flush()
}

// Writes a JSON document to stdout with no styling.
//
// JSON is a machine-readable document, so `--color` must never introduce ANSI
// sequences into it; this bypasses the styler entirely.
pub fn write_json(document: &str) -> io::Result<()> {
    let mut out = std::io::stdout().lock();
    out.write_all(document.as_bytes())?;
    out.flush()
}

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

// Colors diff lines by kind.
//
// The `---`/`+++` file headers share their prefixes with deletions and
// insertions, so a small state machine marks the two lines that structurally
// follow each `diff --ownai` line as headers instead of content (section 13).
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

fn push_styled(out: &mut String, style: Style, text: &str) {
    // Writing to a `String` cannot fail, so the result is intentionally ignored
    // rather than unwrapped.
    let _ = write!(out, "{}{}{}", style.render(), text, style.render_reset());
}
