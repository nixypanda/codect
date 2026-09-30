use base::DiffRowKind;
use ratatui::Frame;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use crate::app::App;
use crate::page::{Loaded, SearchSide};
use crate::render::block::pane_block;
use crate::render::highlight::{self, Run};
use crate::render::layout::{Edge, VisualRow, VisualRowKind, gutter_width};
use crate::render::text::truncate_ellipsis;
use crate::render::theme::Theme;

use crate::render::empty::render_empty;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Side {
    Old,
    New,
}

pub(crate) fn render_diff_pane(
    app: &App,
    frame: &mut Frame,
    area: Rect,
    side: Side,
    focused: bool,
    rows: &[VisualRow],
    edge: Edge,
) {
    let theme = &app.chrome.theme;
    let Loaded::Diff(diff) = &app.loaded else {
        return;
    };
    let (base, target) = diff.revisions();
    let revision = match side {
        Side::Old => base,
        Side::New => target,
    };
    let title = pane_title(side, &revision, area.width);
    let block = pane_block(&title, focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(active) = diff.active_diff() else {
        let (title, detail) = if app.is_busy() && app.tree.rows.is_empty() {
            ("Loading", "Comparing projections…")
        } else if app.tree.rows.is_empty() {
            ("No changes", "Body-only edits are omitted.")
        } else {
            ("No file selected", "Select a file to view its diff.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    };
    if rows.is_empty() {
        render_empty(
            frame,
            inner,
            "No text changes",
            "No projected text to compare for this file.",
            theme,
        );
        return;
    }
    let gutter = gutter_width(active);
    let content_width = (inner.width as usize).saturating_sub(gutter);
    let height = inner.height as usize;
    let skip = diff.body_scroll() as usize;

    let mut lines = Vec::new();
    for row in rows.iter().skip(skip).take(height) {
        let (number, runs) = match side {
            Side::Old => (row.old_number, row.old_runs.as_slice()),
            Side::New => (row.new_number, row.new_runs.as_slice()),
        };
        let ranges = if row.continuation {
            Vec::new()
        } else {
            number.map_or_else(Vec::new, |line| diff.search_ranges(search_side(side), line))
        };
        lines.push(diff_line(
            number,
            row.continuation,
            runs,
            row.kind,
            side,
            gutter,
            content_width,
            theme,
            &ranges,
        ));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}

fn pane_title(side: Side, revision: &str, pane_width: u16) -> String {
    let label = match side {
        Side::Old => "BASE",
        Side::New => "TARGET",
    };
    // Leave room for the rounded border corners on a standalone pane. This
    // also keeps the title within the border in shared-divider layouts.
    let available = (pane_width as usize).saturating_sub(2);
    truncate_ellipsis(&format!(" {label} · {revision} "), available)
}

fn search_side(side: Side) -> SearchSide {
    match side {
        Side::Old => SearchSide::Old,
        Side::New => SearchSide::New,
    }
}

#[allow(clippy::too_many_arguments)]
fn diff_line(
    number: Option<usize>,
    continuation: bool,
    runs: &[Run],
    kind: VisualRowKind,
    side: Side,
    gutter: usize,
    width: usize,
    theme: &Theme,
    search: &[(usize, usize, bool)],
) -> Line<'static> {
    let VisualRowKind::Diff(kind) = kind else {
        // Hunk headers and collapse indicators span the whole pane.
        let text: String = runs.iter().map(|run| run.text.as_str()).collect();
        let full = gutter + width;
        let mut padded = if kind == VisualRowKind::Hunk {
            format!(" {text}")
        } else {
            text
        };
        let used = UnicodeWidthStr::width(padded.as_str());
        if used < full {
            padded.push_str(&" ".repeat(full - used));
        }
        let style = match kind {
            VisualRowKind::Hunk if theme.colors_enabled() => theme
                .fg_bg(theme.palette.hunk, theme.palette.surface_alt)
                .add_modifier(Modifier::BOLD),
            VisualRowKind::Hunk => Style::default(),
            _ => theme.fg(theme.palette.text_muted),
        };
        return Line::from(Span::styled(padded, style));
    };

    // Delta-style full-line background; the intra-line emphasis is already
    // baked into the run styles, so a run's own background wins over the base.
    let base = line_background(kind, side, theme);

    // Search matches are recolored on top of the diff emphasis.
    let owned;
    let runs: &[Run] = if search.is_empty() {
        runs
    } else {
        let mut styled = runs.to_vec();
        for (start, end, current) in search {
            let color = if *current {
                theme.color(theme.palette.match_current_bg)
            } else {
                theme.color(theme.palette.match_bg)
            };
            styled = highlight::apply_emphasis(&styled, &[(*start, *end)], color);
        }
        owned = styled;
        &owned
    };

    let field = gutter.saturating_sub(2);
    let sign = match (kind, side) {
        (DiffRowKind::Add, Side::New) | (DiffRowKind::Change, Side::New) => "+",
        (DiffRowKind::Delete, Side::Old) | (DiffRowKind::Change, Side::Old) => "-",
        _ => " ",
    };
    let number = match number {
        Some(number) => format!("{number:>field$}"),
        None if continuation => format!("{:>field$}", "…"),
        None => " ".repeat(field),
    };
    let sign_color = match sign {
        "+" => theme.palette.add_fg,
        "-" => theme.palette.del_fg,
        _ => theme.palette.gutter,
    };
    let mut marker_style = theme.fg(sign_color);
    if marker_style.bg.is_none() {
        marker_style.bg = base.bg;
    }
    let mut spans = vec![Span::styled(format!("{sign}{number} "), marker_style)];
    let mut used = 0usize;
    for run in runs {
        used += UnicodeWidthStr::width(run.text.as_str());
        let mut style = run.style;
        if style.bg.is_none() {
            style.bg = base.bg;
        }
        spans.push(Span::styled(run.text.clone(), style));
    }
    let padding = width.saturating_sub(used);
    if padding > 0 {
        spans.push(Span::styled(" ".repeat(padding), base));
    }
    Line::from(spans)
}

fn line_background(kind: DiffRowKind, side: Side, theme: &Theme) -> Style {
    if !theme.colors_enabled() {
        return Style::default();
    }
    match (kind, side) {
        (DiffRowKind::Add, _) | (DiffRowKind::Change, Side::New) => theme.bg(theme.palette.add_bg),
        (DiffRowKind::Delete, _) | (DiffRowKind::Change, Side::Old) => {
            theme.bg(theme.palette.del_bg)
        }
        _ => Style::default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::render::theme::{Capability, Flavor};

    #[test]
    fn pane_titles_identify_sides_and_fit_narrow_borders() {
        assert_eq!(pane_title(Side::Old, "main", 20), " BASE · main ");
        assert_eq!(pane_title(Side::New, "feature", 20), " TARGET · feature ");

        let narrow = pane_title(Side::New, "a-long-feature-branch", 16);
        assert_eq!(narrow, " TARGET · a-l…");
        assert_eq!(UnicodeWidthStr::width(narrow.as_str()), 14);
        assert_eq!(pane_title(Side::Old, "main", 2), "");
    }

    #[test]
    fn hunk_header_fills_the_pane_with_a_subtle_band() {
        let theme = Theme::dark();
        let runs = vec![Run {
            style: Style::default(),
            text: "@@ -12,3 +12,4 @@".to_owned(),
        }];

        let line = diff_line(
            None,
            false,
            &runs,
            VisualRowKind::Hunk,
            Side::Old,
            4,
            24,
            &theme,
            &[],
        );
        let span = &line.spans[0];
        assert_eq!(UnicodeWidthStr::width(span.content.as_ref()), 28);
        assert!(span.content.starts_with(" @@ -12,3 +12,4 @@"));
        assert_eq!(span.style.fg, Some(theme.color(theme.palette.hunk)));
        assert_eq!(span.style.bg, Some(theme.color(theme.palette.surface_alt)));
    }

    #[test]
    fn hunk_header_keeps_its_range_markers_without_color() {
        let theme = Theme::new(Flavor::Dark, Capability::NoColor);
        let runs = vec![Run {
            style: Style::default(),
            text: "@@ -1,2 +1,3 @@".to_owned(),
        }];

        let line = diff_line(
            None,
            false,
            &runs,
            VisualRowKind::Hunk,
            Side::New,
            4,
            20,
            &theme,
            &[],
        );
        assert!(line.spans[0].content.starts_with(" @@ -1,2 +1,3 @@"));
        assert_eq!(line.spans[0].style, Style::default());
    }
}
