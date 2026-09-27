//! Design tokens for the terminal frontend.
//!
//! Every color the frontend draws comes from a [`Palette`] resolved through a
//! [`Capability`], so the UI can be themed (dark/light) and degrade gracefully
//! on terminals without truecolor. The palettes and color math are pure data;
//! only [`Theme::detect`] touches the environment and the terminal, and the
//! runtime calls it once at startup before the event reader exists.

use std::sync::OnceLock;
use std::time::Duration;

use ratatui::style::{Color, Style};

/// Whether color is emitted at all.
///
/// The `NO_COLOR` convention disables all styling; text, markers, line numbers,
/// and borders still render so structure survives.
pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

/// The environment variable that pins the flavor. Unset, empty, or `auto` asks
/// the terminal for its background color.
const FLAVOR_ENV: &str = "OWNAI_THEME";

/// The longest the startup background query waits before falling back to dark.
///
/// Terminals that cannot answer are detected almost immediately; the budget only
/// matters for a terminal that answers slowly.
const QUERY_TIMEOUT: Duration = Duration::from_millis(250);

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
    /// The search-match indicator in the status bar and palette, drawn on a
    /// surface rather than on a match highlight.
    pub match_fg: Rgb,
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

/// Tokyo Night night and day colors from folke/tokyonight.nvim:
/// https://github.com/folke/tokyonight.nvim/tree/main/extras/lua
/// Diff fills use its `diff` and `git` colors. Focus uses the clearest accent;
/// directory labels and hunk headers use quieter blue/cyan tones.
const DARK: Palette = Palette {
    bg: Rgb(0x1a, 0x1b, 0x26),
    surface: Rgb(0x16, 0x16, 0x1e),
    surface_alt: Rgb(0x29, 0x2e, 0x42),
    border: Rgb(0x54, 0x5c, 0x7e),
    border_focus: Rgb(0x7d, 0xcf, 0xff),
    divider: Rgb(0x3b, 0x42, 0x61),
    text: Rgb(0xc0, 0xca, 0xf5),
    text_dim: Rgb(0xa9, 0xb1, 0xd6),
    text_muted: Rgb(0x73, 0x7a, 0xa2),
    accent: Rgb(0x7d, 0xcf, 0xff),
    danger: Rgb(0xf7, 0x76, 0x8e),
    selection_bg: Rgb(0x28, 0x34, 0x57),
    selection_fg: Rgb(0xc0, 0xca, 0xf5),
    selection_bar: Rgb(0x7d, 0xcf, 0xff),
    match_bg: Rgb(0x3d, 0x59, 0xa1),
    match_current_bg: Rgb(0xe0, 0xaf, 0x68),
    match_fg: Rgb(0xe0, 0xaf, 0x68),
    guide: Rgb(0x3b, 0x42, 0x61),
    dir: Rgb(0x41, 0xa6, 0xb5),
    file: Rgb(0xc0, 0xca, 0xf5),
    badge_add: Rgb(0x9e, 0xce, 0x6a),
    badge_mod: Rgb(0xe0, 0xaf, 0x68),
    badge_del: Rgb(0xf7, 0x76, 0x8e),
    hunk: Rgb(0x7a, 0xa2, 0xf7),
    gutter: Rgb(0x54, 0x5c, 0x7e),
    del_bg: Rgb(0x4a, 0x27, 0x2f),
    del_emph: Rgb(0x91, 0x4c, 0x54),
    add_bg: Rgb(0x24, 0x3e, 0x4a),
    add_emph: Rgb(0x44, 0x9d, 0xab),
    add_fg: Rgb(0x9e, 0xce, 0x6a),
    del_fg: Rgb(0xf7, 0x76, 0x8e),
};

const LIGHT: Palette = Palette {
    bg: Rgb(0xe1, 0xe2, 0xe7),
    surface: Rgb(0xd0, 0xd5, 0xe3),
    surface_alt: Rgb(0xc4, 0xc8, 0xda),
    border: Rgb(0x89, 0x90, 0xb3),
    border_focus: Rgb(0x2e, 0x7d, 0xe9),
    divider: Rgb(0xa8, 0xae, 0xcb),
    text: Rgb(0x37, 0x60, 0xbf),
    text_dim: Rgb(0x61, 0x72, 0xb0),
    text_muted: Rgb(0x68, 0x70, 0x9a),
    accent: Rgb(0x2e, 0x7d, 0xe9),
    danger: Rgb(0xc6, 0x43, 0x43),
    selection_bg: Rgb(0xb7, 0xc1, 0xe3),
    selection_fg: Rgb(0x37, 0x60, 0xbf),
    selection_bar: Rgb(0x2e, 0x7d, 0xe9),
    match_bg: Rgb(0xd5, 0xd9, 0xe4),
    match_current_bg: Rgb(0x8c, 0x6c, 0x3e),
    match_fg: Rgb(0x8c, 0x6c, 0x3e),
    guide: Rgb(0xa8, 0xae, 0xcb),
    dir: Rgb(0x00, 0x71, 0x97),
    file: Rgb(0x37, 0x60, 0xbf),
    badge_add: Rgb(0x58, 0x75, 0x39),
    badge_mod: Rgb(0x8c, 0x6c, 0x3e),
    badge_del: Rgb(0xc6, 0x43, 0x43),
    hunk: Rgb(0x18, 0x80, 0x92),
    gutter: Rgb(0x89, 0x90, 0xb3),
    del_bg: Rgb(0xda, 0xba, 0xbe),
    del_emph: Rgb(0xc4, 0x79, 0x81),
    add_bg: Rgb(0xb7, 0xce, 0xd5),
    add_emph: Rgb(0x41, 0x97, 0xa4),
    add_fg: Rgb(0x58, 0x75, 0x39),
    del_fg: Rgb(0xc6, 0x43, 0x43),
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

    /// Detects the theme from the environment and the terminal: `OWNAI_THEME`
    /// pins the flavor, and `NO_COLOR`/`COLORTERM`/`TERM` choose the capability.
    /// Called once by the runtime before the event reader starts; `view` stays
    /// pure by reading the theme off the model.
    pub fn detect() -> Self {
        Self::new(detect_flavor(), detect_capability())
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

    /// The palette ink that stays legible on a filled background.
    ///
    /// Chips, badges, and cursors fill a cell with a palette color and then draw
    /// text on it. `palette.bg` is a good ink in the dark flavor because it is
    /// near-black, but in the light flavor it is near-white and vanishes on light
    /// fills such as `surface_alt`. Choosing whichever of `text` or `bg` contrasts
    /// more with the fill keeps both flavors legible.
    pub fn ink(&self, fill: Rgb) -> Rgb {
        let fill = relative_luminance(fill);
        let text = contrast_ratio(fill, relative_luminance(self.palette.text));
        let background = contrast_ratio(fill, relative_luminance(self.palette.bg));
        if text >= background {
            self.palette.text
        } else {
            self.palette.bg
        }
    }

    /// Whether any color is emitted.
    pub fn colors_enabled(&self) -> bool {
        self.capability != Capability::NoColor
    }
}

/// The flavor the frontend draws with.
///
/// `OWNAI_THEME=dark` and `OWNAI_THEME=light` are explicit and skip the query.
/// Any other value (or an unset variable) asks the terminal for its background
/// color and falls back to dark when the terminal does not answer.
pub fn detect_flavor() -> Flavor {
    explicit_flavor(std::env::var(FLAVOR_ENV).ok().as_deref())
        .or_else(query_flavor)
        .unwrap_or(Flavor::Dark)
}

/// The explicit flavor named by `OWNAI_THEME`, if it names one.
///
/// `auto`, an empty value, and an unset variable all defer to
/// [`query_flavor`]; so does any unrecognized value, which keeps a typo from
/// forcing a wrong palette.
fn explicit_flavor(value: Option<&str>) -> Option<Flavor> {
    match value {
        Some("light") => Some(Flavor::Light),
        Some("dark") => Some(Flavor::Dark),
        _ => None,
    }
}

/// Asks the terminal for its background color over `OSC 11`.
///
/// `terminal-colorsaurus` owns the query, saves and restores raw mode itself,
/// and short-circuits terminals that cannot answer, so this returns `None`
/// rather than blocking when detection is unsupported. It is called once, by
/// [`Theme::detect`], before the event reader is first polled, so the reply is
/// not competing with the event loop for standard input.
fn query_flavor() -> Option<Flavor> {
    if !colors_enabled() {
        return None;
    }
    let mut options = terminal_colorsaurus::QueryOptions::default();
    options.timeout = QUERY_TIMEOUT;
    match terminal_colorsaurus::theme_mode(options) {
        Ok(terminal_colorsaurus::ThemeMode::Dark) => Some(Flavor::Dark),
        Ok(terminal_colorsaurus::ThemeMode::Light) => Some(Flavor::Light),
        Err(_) => None,
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

/// The WCAG relative luminance of a color, in `0.0..=1.0`.
fn relative_luminance(rgb: Rgb) -> f32 {
    fn linear(channel: u8) -> f32 {
        let channel = f32::from(channel) / 255.0;
        if channel <= 0.039_28 {
            channel / 12.92
        } else {
            ((channel + 0.055) / 1.055).powf(2.4)
        }
    }
    0.212_6 * linear(rgb.0) + 0.715_2 * linear(rgb.1) + 0.072_2 * linear(rgb.2)
}

/// The WCAG contrast ratio between two relative luminances, from 1.0 to 21.0.
fn contrast_ratio(a: f32, b: f32) -> f32 {
    let (hi, lo) = if a >= b { (a, b) } else { (b, a) };
    (hi + 0.05) / (lo + 0.05)
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
    fn flavors_use_tokyo_night_colors() {
        let dark = Theme::dark().palette;
        let light = Theme::new(Flavor::Light, Capability::TrueColor).palette;
        assert_eq!(dark.bg, Rgb(0x1a, 0x1b, 0x26));
        assert_eq!(dark.add_bg, Rgb(0x24, 0x3e, 0x4a));
        assert_eq!(dark.del_bg, Rgb(0x4a, 0x27, 0x2f));
        assert_eq!(light.bg, Rgb(0xe1, 0xe2, 0xe7));
        assert_eq!(light.add_bg, Rgb(0xb7, 0xce, 0xd5));
        assert_eq!(light.del_bg, Rgb(0xda, 0xba, 0xbe));
        assert_ne!(dark.accent, dark.dir);
        assert_ne!(dark.accent, dark.hunk);
        assert_ne!(light.accent, light.dir);
        assert_ne!(light.accent, light.hunk);
    }

    #[test]
    fn flavors_differ() {
        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        assert_ne!(Theme::dark().palette.bg, light.palette.bg);
    }

    #[test]
    fn explicit_flavor_names_the_two_known_values() {
        assert_eq!(explicit_flavor(Some("dark")), Some(Flavor::Dark));
        assert_eq!(explicit_flavor(Some("light")), Some(Flavor::Light));
    }

    #[test]
    fn auto_and_unknown_values_defer_to_detection() {
        assert_eq!(explicit_flavor(None), None);
        assert_eq!(explicit_flavor(Some("")), None);
        assert_eq!(explicit_flavor(Some("auto")), None);
        // A typo must not force the wrong palette.
        assert_eq!(explicit_flavor(Some("daerk")), None);
    }

    #[test]
    fn ink_contrasts_with_the_fill() {
        let dark = Theme::dark();
        // A bright fill takes the dark ink; a dark fill takes the light ink.
        assert_eq!(dark.ink(dark.palette.accent), dark.palette.bg);
        assert_eq!(dark.ink(dark.palette.bg), dark.palette.text);

        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        assert_eq!(light.ink(Rgb(0xff, 0xff, 0xff)), light.palette.text);
        assert_eq!(light.ink(Rgb(0x00, 0x00, 0x00)), light.palette.bg);
    }

    #[test]
    fn light_ink_keeps_contrast_on_every_chip_fill() {
        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        // The old chip ink was `palette.bg` (near-white), which vanished on the
        // pale `surface_alt` fill; that one must now take the dark ink.
        assert_eq!(light.ink(light.palette.surface_alt), light.palette.text);
        for fill in [
            light.palette.surface_alt,
            light.palette.accent,
            light.palette.badge_add,
            light.palette.badge_mod,
            light.palette.badge_del,
        ] {
            let ink = light.ink(fill);
            let ratio = contrast_ratio(relative_luminance(ink), relative_luminance(fill));
            assert!(
                ratio >= 3.0,
                "ink {ink:?} on fill {fill:?} has contrast {ratio:.2}"
            );
        }
    }
}
