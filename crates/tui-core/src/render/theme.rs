//! Design tokens for the terminal frontend.
//!
// Every color the frontend draws comes from a [`Palette`] resolved through a
// [`Capability`], so the UI can be themed (dark/light) and degrade gracefully
// on terminals without truecolor. The palettes and color math are pure data;
// only [`Theme::detect`] touches the environment and the terminal, and the
// runtime calls it once at startup before the event reader exists.

use std::sync::OnceLock;
use std::time::Duration;

use ratatui::style::{Color, Style};

// Whether color is emitted at all.
//
// The `NO_COLOR` convention disables all styling; text, markers, line numbers,
// and borders still render so structure survives.
pub fn colors_enabled() -> bool {
    std::env::var_os("NO_COLOR").is_none()
}

const FLAVOR_ENV: &str = "CODECT_THEME";

// The longest the startup background query waits before falling back to dark.
//
// Terminals that cannot answer are detected almost immediately; the budget only
// matters for a terminal that answers slowly.
const QUERY_TIMEOUT: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Rgb(pub u8, pub u8, pub u8);

impl Rgb {
    pub fn to_color(self, capability: Capability) -> Color {
        match capability {
            Capability::TrueColor => Color::Rgb(self.0, self.1, self.2),
            Capability::Ansi256 => Color::Indexed(nearest_256(self)),
            Capability::Ansi16 => ANSI16[nearest_ansi16(self)].1,
            Capability::NoColor => Color::Reset,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Capability {
    TrueColor,
    Ansi256,
    Ansi16,
    NoColor,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Flavor {
    Dark,
    Light,
}

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgb,
    pub surface: Rgb,
    pub surface_alt: Rgb,
    pub border: Rgb,
    pub border_focus: Rgb,
    pub divider: Rgb,
    pub text: Rgb,
    pub text_dim: Rgb,
    pub text_muted: Rgb,
    pub accent: Rgb,
    pub danger: Rgb,
    pub selection_bg: Rgb,
    pub selection_fg: Rgb,
    pub selection_bar: Rgb,
    pub match_bg: Rgb,
    pub match_current_bg: Rgb,
    pub match_fg: Rgb,
    pub guide: Rgb,
    pub dir: Rgb,
    pub file: Rgb,
    pub badge_add: Rgb,
    pub badge_mod: Rgb,
    pub badge_del: Rgb,
    pub hunk: Rgb,
    pub gutter: Rgb,
    pub del_bg: Rgb,
    pub del_emph: Rgb,
    pub add_bg: Rgb,
    pub add_emph: Rgb,
    pub add_fg: Rgb,
    pub del_fg: Rgb,
}

// GitHub-inspired palettes: neutral canvases and text, blue focus accents,
// and green/red diff fills. All fills are opaque for consistent terminal output.
const DARK: Palette = Palette {
    bg: Rgb(0x0d, 0x11, 0x17),
    surface: Rgb(0x16, 0x1b, 0x22),
    surface_alt: Rgb(0x21, 0x26, 0x2d),
    border: Rgb(0x30, 0x36, 0x3d),
    border_focus: Rgb(0x58, 0xa6, 0xff),
    divider: Rgb(0x21, 0x26, 0x2d),
    text: Rgb(0xe6, 0xed, 0xf3),
    text_dim: Rgb(0xb1, 0xba, 0xc4),
    text_muted: Rgb(0x8b, 0x94, 0x9e),
    accent: Rgb(0x58, 0xa6, 0xff),
    danger: Rgb(0xff, 0x7b, 0x72),
    selection_bg: Rgb(0x1f, 0x35, 0x50),
    selection_fg: Rgb(0xe6, 0xed, 0xf3),
    selection_bar: Rgb(0x58, 0xa6, 0xff),
    match_bg: Rgb(0x54, 0x3a, 0x0b),
    match_current_bg: Rgb(0xd2, 0x99, 0x22),
    match_fg: Rgb(0xe3, 0xb3, 0x41),
    guide: Rgb(0x30, 0x36, 0x3d),
    dir: Rgb(0x79, 0xc0, 0xff),
    file: Rgb(0xe6, 0xed, 0xf3),
    badge_add: Rgb(0x3f, 0xb9, 0x50),
    badge_mod: Rgb(0xd2, 0x99, 0x22),
    badge_del: Rgb(0xff, 0x7b, 0x72),
    hunk: Rgb(0x79, 0xc0, 0xff),
    gutter: Rgb(0x8b, 0x94, 0x9e),
    del_bg: Rgb(0x2d, 0x16, 0x19),
    del_emph: Rgb(0x67, 0x28, 0x2d),
    add_bg: Rgb(0x12, 0x26, 0x1e),
    add_emph: Rgb(0x1f, 0x4b, 0x32),
    add_fg: Rgb(0x3f, 0xb9, 0x50),
    del_fg: Rgb(0xff, 0x7b, 0x72),
};

const LIGHT: Palette = Palette {
    bg: Rgb(0xff, 0xff, 0xff),
    surface: Rgb(0xf6, 0xf8, 0xfa),
    surface_alt: Rgb(0xea, 0xee, 0xf2),
    border: Rgb(0xd0, 0xd7, 0xde),
    border_focus: Rgb(0x09, 0x69, 0xda),
    divider: Rgb(0xd8, 0xde, 0xe4),
    text: Rgb(0x1f, 0x23, 0x28),
    text_dim: Rgb(0x42, 0x4a, 0x53),
    text_muted: Rgb(0x65, 0x6d, 0x76),
    accent: Rgb(0x09, 0x69, 0xda),
    danger: Rgb(0xcf, 0x22, 0x2e),
    selection_bg: Rgb(0xdd, 0xf4, 0xff),
    selection_fg: Rgb(0x1f, 0x23, 0x28),
    selection_bar: Rgb(0x09, 0x69, 0xda),
    match_bg: Rgb(0xff, 0xf8, 0xc5),
    match_current_bg: Rgb(0xfa, 0xe1, 0x7d),
    match_fg: Rgb(0x7d, 0x4e, 0x00),
    guide: Rgb(0xd8, 0xde, 0xe4),
    dir: Rgb(0x09, 0x69, 0xda),
    file: Rgb(0x1f, 0x23, 0x28),
    badge_add: Rgb(0x1a, 0x7f, 0x37),
    badge_mod: Rgb(0x9a, 0x67, 0x00),
    badge_del: Rgb(0xcf, 0x22, 0x2e),
    hunk: Rgb(0x09, 0x69, 0xda),
    gutter: Rgb(0x65, 0x6d, 0x76),
    del_bg: Rgb(0xff, 0xeb, 0xe9),
    del_emph: Rgb(0xff, 0xce, 0xcb),
    add_bg: Rgb(0xda, 0xfb, 0xe1),
    add_emph: Rgb(0xac, 0xee, 0xbb),
    add_fg: Rgb(0x1a, 0x7f, 0x37),
    del_fg: Rgb(0xcf, 0x22, 0x2e),
};

#[derive(Clone, Copy, Debug)]
pub struct Theme {
    pub flavor: Flavor,
    pub capability: Capability,
    pub palette: Palette,
}

impl Theme {
    #[cfg(any(test, feature = "testing"))]
    pub fn dark() -> Self {
        Self {
            flavor: Flavor::Dark,
            capability: Capability::TrueColor,
            palette: DARK,
        }
    }

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

    pub fn detect() -> Self {
        Self::new(detect_flavor(), detect_capability())
    }

    pub fn color(&self, rgb: Rgb) -> Color {
        rgb.to_color(self.capability)
    }

    pub fn fg(&self, rgb: Rgb) -> Style {
        Style::default().fg(self.color(rgb))
    }

    pub fn bg(&self, rgb: Rgb) -> Style {
        Style::default().bg(self.color(rgb))
    }

    pub fn fg_bg(&self, fg: Rgb, bg: Rgb) -> Style {
        Style::default().fg(self.color(fg)).bg(self.color(bg))
    }

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

    pub fn colors_enabled(&self) -> bool {
        self.capability != Capability::NoColor
    }
}

pub fn detect_flavor() -> Flavor {
    explicit_flavor(std::env::var(FLAVOR_ENV).ok().as_deref())
        .or_else(query_flavor)
        .unwrap_or(Flavor::Dark)
}

fn explicit_flavor(value: Option<&str>) -> Option<Flavor> {
    match value {
        Some("light") => Some(Flavor::Light),
        Some("dark") => Some(Flavor::Dark),
        _ => None,
    }
}

// Asks the terminal for its background color over `OSC 11`.
//
// `terminal-colorsaurus` owns the query, saves and restores raw mode itself,
// and short-circuits terminals that cannot answer, so this returns `None`
// rather than blocking when detection is unsupported. It is called once, by
// [`Theme::detect`], before the event reader is first polled, so the reply is
// not competing with the event loop for standard input.
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
    fn explicit_flavor_names_the_two_known_values() {
        assert_eq!(explicit_flavor(Some("dark")), Some(Flavor::Dark));
        assert_eq!(explicit_flavor(Some("light")), Some(Flavor::Light));
    }

    #[test]
    fn auto_and_unknown_values_defer_to_detection() {
        assert_eq!(explicit_flavor(None), None);
        assert_eq!(explicit_flavor(Some("")), None);
        assert_eq!(explicit_flavor(Some("auto")), None);
        assert_eq!(explicit_flavor(Some("daerk")), None);
    }

    #[test]
    fn ink_contrasts_with_the_fill() {
        let dark = Theme::dark();
        assert_eq!(dark.ink(dark.palette.accent), dark.palette.bg);
        assert_eq!(dark.ink(dark.palette.bg), dark.palette.text);

        let light = Theme::new(Flavor::Light, Capability::TrueColor);
        assert_eq!(light.ink(Rgb(0xff, 0xff, 0xff)), light.palette.text);
        assert_eq!(light.ink(Rgb(0x00, 0x00, 0x00)), light.palette.bg);
    }

    #[test]
    fn light_ink_keeps_contrast_on_every_chip_fill() {
        let light = Theme::new(Flavor::Light, Capability::TrueColor);
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
