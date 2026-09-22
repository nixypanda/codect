//! Design tokens for the terminal frontend.
//!
//! Every color the frontend draws comes from a [`Palette`] resolved through a
//! [`Capability`], so the UI can be themed (dark/light) and degrade gracefully
//! on terminals without truecolor. This module is pure data and color math: it
//! never reads the terminal, and the environment is consulted only by
//! [`Theme::detect`], which the runtime calls once at startup.

use std::sync::OnceLock;

use ratatui::style::{Color, Style};

/// Whether color is emitted at all.
///
/// The `NO_COLOR` convention disables all styling; text, markers, line numbers,
/// and borders still render so structure survives.
pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// An sRGB color used by a palette.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    /// Resolves this color for a terminal of the given capability.
    pub fn to_color(self, capability: Capability) -> Color {
        match capability {
            Capability::TrueColor => Color::Rgb(self.0, self.1, self.2),
            Capability::Ansi256 => Color::Indexed(nearest_256(self)),
            Capability::Ansi16 => ANSI16[nearest_ansi16(self)].1,
            Capability::NoColor => Color::Reset,
        }
    }
}

/// What color a terminal can display.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    /// 24-bit color.
    TrueColor,
    /// The 256-color xterm palette.
    Ansi256,
    /// The 16 basic ANSI colors.
    Ansi16,
    /// No color; `NO_COLOR` is set.
    NoColor,
}

/// A light or dark theme flavor.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flavor {
    Dark,
    Light,
}

/// The semantic colors of one flavor. Views never name a raw color; they name a
/// role from this palette.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// The application background.
    pub bg: Rgb,
    /// A raised panel surface.
    pub surface: Rgb,
    /// A second surface, used for alternating rows and gutters.
    pub surface_alt: Rgb,
    /// An unfocused border.
    pub border: Rgb,
    /// A focused border.
    pub border_focus: Rgb,
    /// A separator between two regions.
    pub divider: Rgb,
    /// Primary text.
    pub text: Rgb,
    /// Secondary text.
    pub text_dim: Rgb,
    /// Tertiary text and disabled affordances.
    pub text_muted: Rgb,
    /// The primary accent.
    pub accent: Rgb,
    /// A negative status.
    pub danger: Rgb,
    /// The selected-row background.
    pub selection_bg: Rgb,
    /// Text on the selected-row background.
    pub selection_fg: Rgb,
    /// The accent bar marking the selected row.
    pub selection_bar: Rgb,
    /// A search match.
    pub match_bg: Rgb,
    /// The current search match.
    pub match_current_bg: Rgb,
    /// A tree guide line.
    pub guide: Rgb,
    /// A directory label.
    pub dir: Rgb,
    /// A file label.
    pub file: Rgb,
    /// An added-file badge.
    pub badge_add: Rgb,
    /// A modified-file badge.
    pub badge_mod: Rgb,
    /// A deleted-file badge.
    pub badge_del: Rgb,
    /// A hunk header.
    pub hunk: Rgb,
    /// A line-number gutter.
    pub gutter: Rgb,
    /// A removed-line background.
    pub del_bg: Rgb,
    /// A removed intra-line emphasis background.
    pub del_emph: Rgb,
    /// An added-line background.
    pub add_bg: Rgb,
    /// An added intra-line emphasis background.
    pub add_emph: Rgb,
    /// Added-line foreground.
    pub add_fg: Rgb,
    /// Removed-line foreground.
    pub del_fg: Rgb,
}

/// Delta's default diff backgrounds are the dark theme's palette values; see
/// [`Palette::del_bg`] and [`Palette::add_bg`].
const DARK: Palette = Palette {
    bg: Rgb(0x1e, 0x1f, 0x1c),
    surface: Rgb(0x27, 0x28, 0x22),
    surface_alt: Rgb(0x2f, 0x30, 0x29),
    border: Rgb(0x49, 0x48, 0x3e),
    border_focus: Rgb(0x66, 0xd9, 0xef),
    divider: Rgb(0x49, 0x48, 0x3e),
    text: Rgb(0xf8, 0xf8, 0xf2),
    text_dim: Rgb(0xa6, 0xa6, 0x9c),
    text_muted: Rgb(0x75, 0x71, 0x5e),
    accent: Rgb(0x66, 0xd9, 0xef),
    danger: Rgb(0xf9, 0x26, 0x72),
    selection_bg: Rgb(0x3e, 0x3d, 0x32),
    selection_fg: Rgb(0xf8, 0xf8, 0xf2),
    selection_bar: Rgb(0xa6, 0xe2, 0x2e),
    match_bg: Rgb(0x66, 0x5c, 0x00),
    match_current_bg: Rgb(0xa6, 0xe2, 0x2e),
    guide: Rgb(0x49, 0x48, 0x3e),
    dir: Rgb(0x66, 0xd9, 0xef),
    file: Rgb(0xf8, 0xf8, 0xf2),
    badge_add: Rgb(0xa6, 0xe2, 0x2e),
    badge_mod: Rgb(0xe6, 0xdb, 0x74),
    badge_del: Rgb(0xf9, 0x26, 0x72),
    hunk: Rgb(0x66, 0xd9, 0xef),
    gutter: Rgb(0x75, 0x71, 0x5e),
    del_bg: Rgb(0x34, 0x00, 0x01),
    del_emph: Rgb(0x64, 0x00, 0x09),
    add_bg: Rgb(0x01, 0x28, 0x00),
    add_emph: Rgb(0x00, 0x60, 0x00),
    add_fg: Rgb(0xa6, 0xe2, 0x2e),
    del_fg: Rgb(0xf9, 0x26, 0x72),
};

const LIGHT: Palette = Palette {
    bg: Rgb(0xf8, 0xf8, 0xf2),
    surface: Rgb(0xff, 0xff, 0xff),
    surface_alt: Rgb(0xee, 0xee, 0xea),
    border: Rgb(0xc8, 0xc8, 0xc0),
    border_focus: Rgb(0x00, 0x87, 0xaf),
    divider: Rgb(0xd0, 0xd0, 0xc8),
    text: Rgb(0x27, 0x28, 0x22),
    text_dim: Rgb(0x6f, 0x6f, 0x66),
    text_muted: Rgb(0x90, 0x90, 0x88),
    accent: Rgb(0x00, 0x87, 0xaf),
    danger: Rgb(0xc7, 0x25, 0x4e),
    selection_bg: Rgb(0xcf, 0xe3, 0xff),
    selection_fg: Rgb(0x10, 0x10, 0x10),
    selection_bar: Rgb(0x00, 0x87, 0xaf),
    match_bg: Rgb(0xff, 0xe0, 0x66),
    match_current_bg: Rgb(0xff, 0xb0, 0x00),
    guide: Rgb(0xc0, 0xc0, 0xb8),
    dir: Rgb(0x00, 0x87, 0xaf),
    file: Rgb(0x27, 0x28, 0x22),
    badge_add: Rgb(0x4f, 0x8f, 0x00),
    badge_mod: Rgb(0xa0, 0x80, 0x00),
    badge_del: Rgb(0xc7, 0x25, 0x4e),
    hunk: Rgb(0x00, 0x87, 0xaf),
    gutter: Rgb(0x90, 0x90, 0x88),
    del_bg: Rgb(0xf8, 0xd7, 0xda),
    del_emph: Rgb(0xf0, 0xa8, 0xb0),
    add_bg: Rgb(0xd6, 0xf5, 0xd6),
    add_emph: Rgb(0xa8, 0xe6, 0xa8),
    add_fg: Rgb(0x2f, 0x6f, 0x00),
    del_fg: Rgb(0xa0, 0x10, 0x30),
};

/// A resolved palette plus the terminal capability it is drawn for.
#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub flavor: Flavor,
    pub capability: Capability,
    pub palette: Palette,
}

impl Theme {
    /// The dark theme at full truecolor, used by tests and previews.
    #[cfg(test)]
    pub fn dark() -> Self {
        Self {
            flavor: Flavor::Dark,
            capability: Capability::TrueColor,
            palette: DARK,
        }
    }

    /// Builds a theme for a flavor and capability.
    pub fn new(flavor: Flavor, capability: Capability) -> Self {
        let palette = match flavor {
            Flavor::Dark => DARK,
            Flavor::Light => LIGHT,
        };
        Self {
            flavor,
            capability,
            palette,
        }
    }

    /// Detects the theme from the environment: `OWNAI_THEME` chooses the flavor
    /// and `NO_COLOR`/`COLORTERM`/`TERM` choose the capability. Called once by
    /// the runtime; `view` stays pure by reading the theme off the model.
    pub fn detect() -> Self {
        let flavor = match std::env::var("OWNAI_THEME").ok().as_deref() {
            Some("light") => Flavor::Light,
            _ => Flavor::Dark,
        };
        Self::new(flavor, detect_capability())
    }

    /// Resolves a palette color for this terminal.
    pub fn color(&self, rgb: Rgb) -> Color {
        rgb.to_color(self.capability)
    }

    /// A foreground style for a palette color.
    pub fn fg(&self, rgb: Rgb) -> Style {
        Style::default().fg(self.color(rgb))
    }

    /// A background style for a palette color.
    pub fn bg(&self, rgb: Rgb) -> Style {
        Style::default().bg(self.color(rgb))
    }

    /// A foreground-and-background style.
    pub fn fg_bg(&self, fg: Rgb, bg: Rgb) -> Style {
        Style::default().fg(self.color(fg)).bg(self.color(bg))
    }

    /// Whether any color is emitted.
    pub fn colors_enabled(&self) -> bool {
        self.capability != Capability::NoColor
    }
}

/// Detects the terminal's color capability from the environment.
pub fn detect_capability() -> Capability {
    if !colors_enabled() {
        return Capability::NoColor;
    }
    if let Some(colorterm) = std::env::var_os("COLORTERM") {
        let colorterm = colorterm.to_string_lossy().to_ascii_lowercase();
        if colorterm.contains("truecolor") || colorterm.contains("24bit") {
            return Capability::TrueColor;
        }
    }
    match std::env::var("TERM").unwrap_or_default() {
        term if term.contains("truecolor") || term.contains("24bit") => Capability::TrueColor,
        term if term.contains("256color") => Capability::Ansi256,
        _ => Capability::Ansi16,
    }
}

/// The 16 basic ANSI colors and their representative sRGB values.
const ANSI16: [(Rgb, Color); 16] = [
    (Rgb(0x00, 0x00, 0x00), Color::Black),
    (Rgb(0x80, 0x00, 0x00), Color::Red),
    (Rgb(0x00, 0x80, 0x00), Color::Green),
    (Rgb(0x80, 0x80, 0x00), Color::Yellow),
    (Rgb(0x00, 0x00, 0x80), Color::Blue),
    (Rgb(0x80, 0x00, 0x80), Color::Magenta),
    (Rgb(0x00, 0x80, 0x80), Color::Cyan),
    (Rgb(0xc0, 0xc0, 0xc0), Color::Gray),
    (Rgb(0x80, 0x80, 0x80), Color::DarkGray),
    (Rgb(0xff, 0x00, 0x00), Color::LightRed),
    (Rgb(0x00, 0xff, 0x00), Color::LightGreen),
    (Rgb(0xff, 0xff, 0x00), Color::LightYellow),
    (Rgb(0x00, 0x00, 0xff), Color::LightBlue),
    (Rgb(0xff, 0x00, 0xff), Color::LightMagenta),
    (Rgb(0x00, 0xff, 0xff), Color::LightCyan),
    (Rgb(0xff, 0xff, 0xff), Color::White),
];

/// The 256-entry xterm palette, built once.
fn xterm256() -> &'static [Rgb; 256] {
    static PALETTE: OnceLock<[Rgb; 256]> = OnceLock::new();
    PALETTE.get_or_init(|| {
        let mut palette = [Rgb(0, 0, 0); 256];
        for (index, (rgb, _)) in ANSI16.iter().enumerate() {
            palette[index] = *rgb;
        }
        const LEVELS: [u8; 6] = [0, 95, 135, 175, 215, 255];
        for r in 0..6usize {
            for g in 0..6usize {
                for b in 0..6usize {
                    palette[16 + 36 * r + 6 * g + b] = Rgb(LEVELS[r], LEVELS[g], LEVELS[b]);
                }
            }
        }
        for step in 0..24usize {
            let level = 8 + 10 * step as u8;
            palette[232 + step] = Rgb(level, level, level);
        }
        palette
    })
}

fn distance_squared(a: Rgb, b: Rgb) -> u32 {
    let dr = i32::from(a.0) - i32::from(b.0);
    let dg = i32::from(a.1) - i32::from(b.1);
    let db = i32::from(a.2) - i32::from(b.2);
    (dr * dr + dg * dg + db * db) as u32
}

fn nearest_256(rgb: Rgb) -> u8 {
    let mut best = 0u8;
    let mut best_distance = u32::MAX;
    for (index, candidate) in xterm256().iter().enumerate() {
        let distance = distance_squared(rgb, *candidate);
        if distance < best_distance {
            best_distance = distance;
            best = index as u8;
        }
    }
    best
}

fn nearest_ansi16(rgb: Rgb) -> usize {
    let mut best = 0usize;
    let mut best_distance = u32::MAX;
    for (index, (candidate, _)) in ANSI16.iter().enumerate() {
        let distance = distance_squared(rgb, *candidate);
        if distance < best_distance {
            best_distance = distance;
            best = index;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn truecolor_is_identity() {
        assert_eq!(
            Rgb(1, 2, 3).to_color(Capability::TrueColor),
            Color::Rgb(1, 2, 3)
        );
    }

    #[test]
    fn no_color_resets_everything() {
        assert_eq!(Rgb(1, 2, 3).to_color(Capability::NoColor), Color::Reset);
    }

    #[test]
    fn ansi16_maps_to_a_named_color() {
        assert_eq!(
            Rgb(0xff, 0x00, 0x00).to_color(Capability::Ansi16),
            Color::LightRed
        );
        assert_eq!(
            Rgb(0x00, 0x00, 0x00).to_color(Capability::Ansi16),
            Color::Black
        );
    }

    #[test]
    fn ansi256_maps_to_an_index() {
        let color = Rgb(0xff, 0xff, 0xff).to_color(Capability::Ansi256);
        assert!(matches!(color, Color::Indexed(_)));
        assert_eq!(color, Color::Indexed(15));
    }

    #[test]
    fn xterm256_has_the_gray_ramp_at_the_end() {
        let palette = xterm256();
        assert_eq!(palette[232], Rgb(8, 8, 8));
        assert_eq!(palette[255], Rgb(238, 238, 238));
    }

    #[test]
    fn dark_theme_keeps_delta_defaults() {
        let theme = Theme::dark();
        assert_eq!(
            theme.color(theme.palette.del_bg),
            Color::Rgb(0x34, 0x00, 0x01)
        );
        assert_eq!(
            theme.color(theme.palette.add_bg),
            Color::Rgb(0x01, 0x28, 0x00)
        );
    }

    #[test]
    fn flavors_differ() {
        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        assert_ne!(Theme::dark().palette.bg, light.palette.bg);
    }
}
