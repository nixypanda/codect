//! Optional Nerd Font glyphs for the file tree.
//!
//! Nerd Fonts patch a terminal font with icon glyphs in the Private Use Area.
//! They cannot be detected reliably from inside a program, so icons are opt-in:
//! with the default [`IconStyle::None`] the tree draws no glyphs and never
//! risks a missing-glyph box. The chosen codepoints come from the classic Font
//! Awesome and Devicons/Seti ranges, which exist in Nerd Fonts v2 and are
//! aliased in v3, so they are the most widely supported.
//!
//! Every glyph here is a single display column, which keeps the tree's
//! display-width alignment intact.

use ownai_core::{Language, RepoPath};

/// How the file tree decorates rows.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IconStyle {
    /// No icons; the tree stays readable on any font.
    None,
    /// Nerd Font folder and file-type glyphs. Requires a Nerd Font.
    Nerd,
}

/// The resolved glyph set. Stored on the model so `view` stays pure.
#[derive(Clone, Copy, Debug)]
pub struct Icons {
    style: IconStyle,
}

impl Icons {
    pub fn new(style: IconStyle) -> Self {
        Self { style }
    }

    /// The glyph drawn before a directory label. The `▾`/`▸` chevron still
    /// carries the open/closed state, so this glyph is static.
    pub fn folder(&self) -> &'static str {
        match self.style {
            IconStyle::None => "",
            IconStyle::Nerd => "\u{f07b}",
        }
    }

    /// The glyph drawn before a file label, chosen by language.
    pub fn file(&self, path: &RepoPath) -> &'static str {
        match self.style {
            IconStyle::None => "",
            IconStyle::Nerd => match path.language() {
                Some(Language::Rust) => "\u{e7a8}",
                Some(Language::Elm) => "\u{e62c}",
                Some(Language::Haskell) => "\u{e777}",
                Some(Language::Python) => "\u{e73c}",
                None => "\u{f15b}",
            },
        }
    }
}

/// Every Nerd glyph the tree can draw, for the width invariant test.
#[cfg(test)]
const NERD_GLYPHS: [&str; 6] = [
    "\u{f07b}", "\u{e7a8}", "\u{e62c}", "\u{e777}", "\u{e73c}", "\u{f15b}",
];

#[cfg(test)]
mod tests {
    use super::*;
    use unicode_width::UnicodeWidthStr;

    fn path(value: &str) -> RepoPath {
        RepoPath::new(value).expect("valid path")
    }

    #[test]
    fn nerd_files_are_chosen_by_language() {
        let icons = Icons::new(IconStyle::Nerd);
        assert_eq!(icons.file(&path("src/main.rs")), "\u{e7a8}");
        assert_eq!(icons.file(&path("src/Main.elm")), "\u{e62c}");
        assert_eq!(icons.file(&path("src/Main.hs")), "\u{e777}");
        assert_eq!(icons.file(&path("src/app.py")), "\u{e73c}");
        assert_eq!(icons.file(&path("README.md")), "\u{f15b}");
    }

    #[test]
    fn every_glyph_is_one_display_column() {
        for glyph in NERD_GLYPHS {
            assert_eq!(
                UnicodeWidthStr::width(glyph),
                1,
                "glyph {glyph:?} must occupy one column"
            );
        }
    }
}
