//! Delta-style syntax highlighting and diff styling for the terminal.
//!
//! This module is presentation-only. It never changes canonical projection
//! text, never touches Git, and never reads the terminal. It turns a plain
//! projection string into ordered [`Run`]s carrying a [`Style`], so the render
//! layer can draw syntax foregrounds and diff backgrounds.
//!
//! Syntax grammars come from `two-face` (bat's syntax bundle). Token colors use
//! Tokyo Night night/day colors so the code agrees with the rest of the UI.
//! Diff backgrounds and intra-line emphasis come from the semantic palette.

use std::sync::OnceLock;

use ownai_core::Language;
use ratatui::style::{Color, Modifier, Style};
use syntect::easy::HighlightLines;
use syntect::highlighting::ScopeSelectors;
use syntect::highlighting::{
    Color as SynColor, FontStyle, Style as SynStyle, StyleModifier, Theme as SynTheme, ThemeItem,
    ThemeSettings,
};
use syntect::parsing::{SyntaxReference, SyntaxSet};
use syntect::util::LinesWithEndings;
use unicode_width::UnicodeWidthChar;

use crate::theme::{Capability, Flavor, Rgb, Theme};

/// Columns a tab expands to, matching the projection renderer.
const TAB_WIDTH: usize = 4;

/// One styled run of text within a rendered line.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Run {
    pub style: Style,
    pub text: String,
}

/// A rendered line as a sequence of styled runs.
pub type StyledLine = Vec<Run>;

struct Assets {
    syntaxes: SyntaxSet,
    dark: SynTheme,
    light: SynTheme,
}

fn assets() -> &'static Assets {
    static ASSETS: OnceLock<Assets> = OnceLock::new();
    ASSETS.get_or_init(|| {
        let syntaxes = two_face::syntax::extra_newlines();
        Assets {
            syntaxes,
            dark: tokyo_night_theme(Flavor::Dark),
            light: tokyo_night_theme(Flavor::Light),
        }
    })
}

fn syn_color(rgb: Rgb) -> SynColor {
    SynColor {
        r: rgb.0,
        g: rgb.1,
        b: rgb.2,
        a: 0xff,
    }
}

/// TextMate scopes shared by the bat grammars. These are built from Tokyo
/// Night's night/day colors rather than carrying a second, unrelated theme.
fn tokyo_night_theme(flavor: Flavor) -> SynTheme {
    let (foreground, background, comment, red, green, yellow, blue, purple, cyan, orange) =
        match flavor {
            Flavor::Dark => (
                Rgb(0xc0, 0xca, 0xf5),
                Rgb(0x1a, 0x1b, 0x26),
                Rgb(0x73, 0x7a, 0xa2),
                Rgb(0xf7, 0x76, 0x8e),
                Rgb(0x9e, 0xce, 0x6a),
                Rgb(0xe0, 0xaf, 0x68),
                Rgb(0x7a, 0xa2, 0xf7),
                Rgb(0xbb, 0x9a, 0xf7),
                Rgb(0x7d, 0xcf, 0xff),
                Rgb(0xff, 0x9e, 0x64),
            ),
            Flavor::Light => (
                Rgb(0x37, 0x60, 0xbf),
                Rgb(0xe1, 0xe2, 0xe7),
                Rgb(0x68, 0x70, 0x9a),
                Rgb(0xc6, 0x43, 0x43),
                Rgb(0x58, 0x75, 0x39),
                Rgb(0x8c, 0x6c, 0x3e),
                Rgb(0x2e, 0x7d, 0xe9),
                Rgb(0x78, 0x47, 0xbd),
                Rgb(0x00, 0x71, 0x97),
                Rgb(0xb1, 0x5c, 0x00),
            ),
        };
    let mut theme = SynTheme {
        name: Some(
            match flavor {
                Flavor::Dark => "Tokyo Night Night",
                Flavor::Light => "Tokyo Night Day",
            }
            .to_owned(),
        ),
        settings: ThemeSettings {
            foreground: Some(syn_color(foreground)),
            background: Some(syn_color(background)),
            ..ThemeSettings::default()
        },
        ..SynTheme::default()
    };
    for (selectors, color) in [
        ("comment", comment),
        ("keyword, storage", purple),
        ("keyword.operator, punctuation.definition", purple),
        ("constant.numeric, constant.language", orange),
        ("string", green),
        ("entity.name.function, support.function", blue),
        ("entity.name.type, entity.name.class, support.type", yellow),
        ("variable.parameter, variable.other.member", red),
        ("entity.name.tag, entity.other.attribute-name", red),
        ("support.constant, constant.other", cyan),
    ] {
        theme.scopes.push(ThemeItem {
            scope: selectors
                .parse::<ScopeSelectors>()
                .expect("valid TextMate scope"),
            style: StyleModifier {
                foreground: Some(syn_color(color)),
                ..StyleModifier::default()
            },
        });
    }
    theme
}

fn syntect_theme(assets: &Assets, flavor: Flavor) -> &SynTheme {
    match flavor {
        Flavor::Dark => &assets.dark,
        Flavor::Light => &assets.light,
    }
}

fn syntax_for(syntaxes: &SyntaxSet, language: Language) -> &SyntaxReference {
    let extension = match language {
        Language::Rust => "rs",
        Language::Elm => "elm",
        Language::Haskell => "hs",
        Language::Python => "py",
    };
    syntaxes
        .find_syntax_by_extension(extension)
        .unwrap_or_else(|| syntaxes.find_syntax_plain_text())
}

fn convert(style: SynStyle, capability: Capability) -> Style {
    let foreground = Rgb(style.foreground.r, style.foreground.g, style.foreground.b);
    let mut out = Style::default().fg(foreground.to_color(capability));
    if style.font_style.contains(FontStyle::BOLD) {
        out = out.add_modifier(Modifier::BOLD);
    }
    if style.font_style.contains(FontStyle::ITALIC) {
        out = out.add_modifier(Modifier::ITALIC);
    }
    if style.font_style.contains(FontStyle::UNDERLINE) {
        out = out.add_modifier(Modifier::UNDERLINED);
    }
    out
}

fn plain_line(line: &str) -> StyledLine {
    vec![Run {
        style: Style::default(),
        text: line.to_owned(),
    }]
}

/// Highlights `text` into one [`StyledLine`] per source line.
///
/// The result has exactly `text.lines().count()` entries, so a caller can index
/// it by one-based source line number minus one. The `theme` selects the syntax
/// flavor and resolves token colors for the terminal's capability. When color is
/// disabled, or a line fails to highlight, a single default-styled run is
/// produced.
pub fn highlight(text: &str, language: Language, theme: &Theme) -> Vec<StyledLine> {
    if !theme.colors_enabled() {
        return text.lines().map(plain_line).collect();
    }

    let assets = assets();
    let syntax = syntax_for(&assets.syntaxes, language);
    let mut highlighter = HighlightLines::new(syntax, syntect_theme(assets, theme.flavor));

    let mut lines = Vec::new();
    for line in LinesWithEndings::from(text) {
        let trimmed = line.trim_end_matches(['\n', '\r']);
        match highlighter.highlight_line(line, &assets.syntaxes) {
            Ok(ranges) => {
                let mut runs = Vec::new();
                for (style, piece) in ranges {
                    let piece = piece.trim_end_matches(['\n', '\r']);
                    if piece.is_empty() {
                        continue;
                    }
                    push_run(&mut runs, convert(style, theme.capability), piece);
                }
                if runs.is_empty() {
                    runs.push(Run {
                        style: Style::default(),
                        text: trimmed.to_owned(),
                    });
                }
                lines.push(runs);
            }
            Err(_) => lines.push(plain_line(trimmed)),
        }
    }

    // `LinesWithEndings` yields no entry for empty text; keep the line count
    // aligned with `str::lines` for every input.
    if lines.len() < text.lines().count() {
        while lines.len() < text.lines().count() {
            lines.push(plain_line(""));
        }
    }
    lines
}

/// Byte ranges within a line that should receive intra-line emphasis.
pub type Emphasis = Vec<(usize, usize)>;

/// The byte ranges that differ between two aligned lines, as a common
/// prefix/suffix trim. This is delta's intra-line emphasis: the unchanged edges
/// stay at the line background, the changed middle gets a brighter one.
///
/// Returns `(old_ranges, new_ranges)`; both are empty when the texts are equal.
pub fn emphasis_ranges(old: &str, new: &str) -> (Emphasis, Emphasis) {
    if old == new {
        return (Vec::new(), Vec::new());
    }

    let old_chars: Vec<(usize, char)> = old.char_indices().collect();
    let new_chars: Vec<(usize, char)> = new.char_indices().collect();

    let mut prefix = 0;
    while prefix < old_chars.len()
        && prefix < new_chars.len()
        && old_chars[prefix].1 == new_chars[prefix].1
    {
        prefix += 1;
    }

    let mut suffix = 0;
    while suffix < old_chars.len().saturating_sub(prefix)
        && suffix < new_chars.len().saturating_sub(prefix)
        && old_chars[old_chars.len() - 1 - suffix].1 == new_chars[new_chars.len() - 1 - suffix].1
    {
        suffix += 1;
    }

    let old_start = old_chars.get(prefix).map_or(old.len(), |(byte, _)| *byte);
    let old_end = old_chars
        .get(old_chars.len() - suffix)
        .map_or(old.len(), |(byte, _)| *byte);
    let new_start = new_chars.get(prefix).map_or(new.len(), |(byte, _)| *byte);
    let new_end = new_chars
        .get(new_chars.len() - suffix)
        .map_or(new.len(), |(byte, _)| *byte);

    let mut old_ranges = Vec::new();
    let mut new_ranges = Vec::new();
    if old_start < old_end {
        old_ranges.push((old_start, old_end));
    }
    if new_start < new_end {
        new_ranges.push((new_start, new_end));
    }
    (old_ranges, new_ranges)
}

/// Recolors the byte ranges `ranges` with `background`, splitting runs as
/// needed. Ranges are assumed to be on character boundaries.
pub fn apply_emphasis(runs: &[Run], ranges: &[(usize, usize)], background: Color) -> StyledLine {
    if ranges.is_empty() {
        return runs.to_vec();
    }

    let mut out: StyledLine = Vec::new();
    let mut offset = 0usize;
    for run in runs {
        let mut piece = String::new();
        let mut piece_style = run.style;
        let mut cursor = offset;
        for character in run.text.chars() {
            let length = character.len_utf8();
            let emphasized = ranges
                .iter()
                .any(|&(start, end)| cursor >= start && cursor < end);
            let style = if emphasized {
                run.style.bg(background)
            } else {
                run.style
            };
            if !piece.is_empty() && style != piece_style {
                push_run(&mut out, piece_style, &piece);
                piece.clear();
            }
            piece_style = style;
            piece.push(character);
            cursor += length;
        }
        if !piece.is_empty() {
            push_run(&mut out, piece_style, &piece);
        }
        offset += run.text.len();
    }
    out
}

/// Splits styled runs into display-width segments, expanding tabs.
///
/// The result has at least one segment, so callers can always index `[0]`.
pub fn wrap_runs(runs: &[Run], width: usize) -> Vec<StyledLine> {
    if width == 0 {
        return vec![Vec::new()];
    }

    let mut segments: Vec<StyledLine> = Vec::new();
    let mut current: StyledLine = Vec::new();
    let mut cells = 0usize;

    for run in runs {
        for character in run.text.chars() {
            if character == '\t' {
                for _ in 0..TAB_WIDTH {
                    wrap_push(
                        &mut current,
                        &mut segments,
                        run.style,
                        ' ',
                        &mut cells,
                        width,
                    );
                }
                continue;
            }
            wrap_push(
                &mut current,
                &mut segments,
                run.style,
                character,
                &mut cells,
                width,
            );
        }
    }
    segments.push(current);
    segments
}

fn wrap_push(
    current: &mut StyledLine,
    segments: &mut Vec<StyledLine>,
    style: Style,
    character: char,
    cells: &mut usize,
    width: usize,
) {
    let character_width = UnicodeWidthChar::width(character).unwrap_or(0);
    if *cells > 0 && *cells + character_width > width {
        segments.push(std::mem::take(current));
        *cells = 0;
    }
    push_run(current, style, &character.to_string());
    *cells += character_width;
}

/// A display-width slice of one styled line, expanding tabs.
///
/// Mirrors the plain-text clipping rules: never split a code point, drop a wide
/// character that straddles the cut whole, and keep combining marks with their
/// base.
pub fn clip_runs(runs: &[Run], skip: usize, width: usize) -> StyledLine {
    if width == 0 {
        return Vec::new();
    }

    let mut out: StyledLine = Vec::new();
    let mut column = 0usize;
    let mut taken = 0usize;

    'outer: for run in runs {
        for character in run.text.chars() {
            let characters: Vec<char> = if character == '\t' {
                std::iter::repeat_n(' ', TAB_WIDTH).collect()
            } else {
                vec![character]
            };
            for character in characters {
                if !clip_push(
                    &mut out,
                    run.style,
                    character,
                    &mut column,
                    &mut taken,
                    skip,
                    width,
                ) {
                    break 'outer;
                }
            }
        }
    }
    out
}

fn clip_push(
    out: &mut StyledLine,
    style: Style,
    character: char,
    column: &mut usize,
    taken: &mut usize,
    skip: usize,
    width: usize,
) -> bool {
    let cells = UnicodeWidthChar::width(character).unwrap_or(0);
    if cells == 0 {
        if *column >= skip {
            push_run(out, style, &character.to_string());
        }
        return true;
    }
    if *column + cells <= skip {
        *column += cells;
        return true;
    }
    if *column < skip {
        *column += cells;
        return true;
    }
    if *taken + cells > width {
        return false;
    }
    push_run(out, style, &character.to_string());
    *taken += cells;
    *column += cells;
    true
}

fn push_run(runs: &mut StyledLine, style: Style, text: &str) {
    if text.is_empty() {
        return;
    }
    if let Some(last) = runs.last_mut()
        && last.style == style
    {
        last.text.push_str(text);
        return;
    }
    runs.push(Run {
        style,
        text: text.to_owned(),
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rust_keywords_get_a_foreground_color() {
        let lines = highlight(
            "pub struct User {\n    id: u32,\n}\n",
            Language::Rust,
            &Theme::dark(),
        );
        assert_eq!(lines.len(), 3, "one styled line per source line");

        // `pub` is a keyword and must not be default-styled.
        let keyword = &lines[0][0];
        assert!(
            keyword.style.fg.is_some(),
            "the first token should carry a syntax foreground"
        );
        let text: String = lines[0].iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "pub struct User {");
    }

    #[test]
    fn syntax_tokens_match_tokyo_night_in_both_flavors() {
        for (flavor, keyword, string) in [
            (
                Flavor::Dark,
                Color::Rgb(0xbb, 0x9a, 0xf7),
                Color::Rgb(0x9e, 0xce, 0x6a),
            ),
            (
                Flavor::Light,
                Color::Rgb(0x78, 0x47, 0xbd),
                Color::Rgb(0x58, 0x75, 0x39),
            ),
        ] {
            let theme = Theme::new(flavor, Capability::TrueColor);
            let lines = highlight(
                "pub fn main() { let name = \"ownai\"; }",
                Language::Rust,
                &theme,
            );
            let keyword_run = lines[0]
                .iter()
                .find(|run| run.text.contains("pub"))
                .unwrap();
            let string_run = lines[0]
                .iter()
                .find(|run| run.text.contains("ownai"))
                .unwrap();
            assert_eq!(keyword_run.style.fg, Some(keyword));
            assert_eq!(string_run.style.fg, Some(string));
        }
    }

    #[test]
    fn syntax_respects_no_color_and_limited_palettes() {
        let text = "pub fn main() {}";
        let plain = highlight(
            text,
            Language::Rust,
            &Theme::new(Flavor::Dark, Capability::NoColor),
        );
        assert_eq!(
            plain,
            vec![vec![Run {
                style: Style::default(),
                text: text.to_owned()
            }]]
        );
        for capability in [Capability::Ansi16, Capability::Ansi256] {
            let lines = highlight(text, Language::Rust, &Theme::new(Flavor::Dark, capability));
            assert!(lines[0].iter().any(|run| run.style.fg.is_some()));
        }
    }

    #[test]
    fn line_count_matches_str_lines() {
        for text in ["", "\n", "a", "a\n", "a\n\nb\n", "a\nb"] {
            assert_eq!(
                highlight(text, Language::Rust, &Theme::dark()).len(),
                text.lines().count(),
                "text = {text:?}"
            );
        }
    }

    #[test]
    fn emphasis_marks_only_the_changed_middle() {
        let (old, new) = emphasis_ranges("pub id: u32;", "pub id: u64;");
        assert_eq!(old, vec![(9, 11)]);
        assert_eq!(new, vec![(9, 11)]);

        let (old, new) = emphasis_ranges("same", "same");
        assert!(old.is_empty() && new.is_empty());
    }

    #[test]
    fn emphasis_survives_a_fully_different_line() {
        let (old, new) = emphasis_ranges("abc", "xyz");
        assert_eq!(old, vec![(0, 3)]);
        assert_eq!(new, vec![(0, 3)]);
    }

    #[test]
    fn apply_emphasis_recolors_only_the_range() {
        let runs = vec![Run {
            style: Style::default().fg(Color::White),
            text: "abcde".to_owned(),
        }];
        let out = apply_emphasis(&runs, &[(1, 3)], Color::Rgb(0x64, 0x00, 0x09));
        let text: String = out.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "abcde");
        assert_eq!(out[0].style.bg, None);
        assert_eq!(out[1].style.bg, Some(Color::Rgb(0x64, 0x00, 0x09)));
        assert_eq!(out[2].style.bg, None);
    }

    #[test]
    fn wrap_runs_preserves_styles_across_a_break() {
        let runs = vec![
            Run {
                style: Style::default().fg(Color::Red),
                text: "aaaa".to_owned(),
            },
            Run {
                style: Style::default().fg(Color::Blue),
                text: "bbbb".to_owned(),
            },
        ];
        let segments = wrap_runs(&runs, 4);
        assert_eq!(segments.len(), 2);
        let first: String = segments[0].iter().map(|run| run.text.as_str()).collect();
        let second: String = segments[1].iter().map(|run| run.text.as_str()).collect();
        assert_eq!(first, "aaaa");
        assert_eq!(second, "bbbb");
        assert_eq!(segments[0][0].style.fg, Some(Color::Red));
        assert_eq!(segments[1][0].style.fg, Some(Color::Blue));
    }

    #[test]
    fn clip_runs_slices_by_display_width_and_keeps_style() {
        let runs = vec![Run {
            style: Style::default().fg(Color::Green),
            text: "abcdef".to_owned(),
        }];
        let out = clip_runs(&runs, 2, 3);
        let text: String = out.iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "cde");
        assert_eq!(out[0].style.fg, Some(Color::Green));
        assert!(clip_runs(&runs, 0, 0).is_empty());
    }

    #[test]
    fn wrap_runs_expands_tabs_to_four_columns() {
        let runs = vec![Run {
            style: Style::default(),
            text: "a\tb".to_owned(),
        }];
        let segments = wrap_runs(&runs, 10);
        let text: String = segments[0].iter().map(|run| run.text.as_str()).collect();
        assert_eq!(text, "a    b");
    }
}
