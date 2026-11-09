//! The diff panes: one side of a file diff, with its gutter and delta styling.

use base::{DiffRowKind, FileDiff, ProjectedFile, RepoPath};
use ratatui::Frame;
use ratatui::layout::{Alignment, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use unicode_width::UnicodeWidthStr;

use tui_core::cx::Cx;
use tui_core::layout::Edge;
use tui_core::render::block::pane_block;
use tui_core::render::empty::render_empty;
use tui_core::render::highlight::{self, Run};
use tui_core::render::text::{elide_path, middle_ellipsis, truncate_ellipsis};
use tui_core::render::theme::Theme;
use tui_core::search::{SearchMatch, SearchPane, current_match};

use super::DiffPlugin;
use super::layout::{VisualRowKind, gutter_width};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Side {
    Old,
    New,
}

#[allow(clippy::too_many_arguments)]
pub fn render(
    model: &DiffPlugin,
    cx: &Cx,
    frame: &mut Frame,
    area: Rect,
    side: Side,
    focused: bool,
    edge: Edge,
) {
    let theme = cx.theme;
    let (base, target) = model.revisions();
    let revision = match side {
        Side::Old => base,
        Side::New => target,
    };
    let active = model.active_diff(cx.workspace.selected.as_ref());
    let moved_path = active.and_then(|diff| moved_path(side, diff));
    let title = pane_title(side, &revision, moved_path, model.merge_base, area.width);
    let block = pane_block(&title, focused, theme, edge);
    let inner = block.inner(area);
    frame.render_widget(block, area);

    let Some(active) = active else {
        let tree_empty = cx.workspace.files.paths.is_empty();
        let (title, detail) = if cx.busy && tree_empty {
            ("Loading", "Comparing projections…")
        } else if tree_empty {
            ("No changes", "Body-only edits are omitted.")
        } else {
            ("No file selected", "Select a file to view its diff.")
        };
        render_empty(frame, inner, title, detail, theme);
        return;
    };
    let rows = model.rows();
    if rows.is_empty() {
        if matches!(active, FileDiff::Moved { .. }) {
            render_move_empty(frame, inner, active, cx, theme);
        } else {
            render_empty(
                frame,
                inner,
                "No text changes",
                "No projected text to compare for this file.",
                theme,
            );
        }
        return;
    }
    let gutter = gutter_width(active);
    let content_width = (inner.width as usize).saturating_sub(gutter);
    let height = inner.height as usize;
    let skip = model.body_scroll() as usize;

    let mut lines = Vec::new();
    for row in rows.iter().skip(skip).take(height) {
        let (number, runs) = match side {
            Side::Old => (row.old_number, row.old_runs.as_slice()),
            Side::New => (row.new_number, row.new_runs.as_slice()),
        };
        let ranges = if row.continuation {
            Vec::new()
        } else {
            number.map_or_else(Vec::new, |line| {
                cx.workspace.ranges_on_line(search_pane(side), line)
            })
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

fn moved_path(side: Side, diff: &FileDiff) -> Option<&RepoPath> {
    match (side, diff) {
        (Side::Old, FileDiff::Moved { old, .. }) => Some(old.path()),
        (Side::New, FileDiff::Moved { new, .. }) => Some(new.path()),
        _ => None,
    }
}

/// The pure-move empty state: where the file came from, where it went, and the
/// current search hit when one exists. The one-line detail is kept when it
/// fits; otherwise the two paths get separate lines so neither tail is lost.
/// A matched projection line gives `n`/`N` visible feedback without fabricating
/// hunk rows.
fn render_move_empty(frame: &mut Frame, area: Rect, diff: &FileDiff, cx: &Cx, theme: &Theme) {
    let FileDiff::Moved { old, new } = diff else {
        return;
    };
    let width = area.width as usize;
    let height = area.height as usize;
    if width == 0 || height == 0 {
        return;
    }

    let mut lines = vec![Line::from(Span::styled(
        "Moved",
        theme.fg(theme.palette.text),
    ))];
    let one_line = format!("Moved from {} to {}.", old.path(), new.path());
    if UnicodeWidthStr::width(one_line.as_str()) <= width {
        lines.push(detail_line(one_line, theme));
    } else {
        lines.push(path_line("from ", old.path(), width, theme));
        lines.push(path_line("to ", new.path(), width, theme));
    }

    if let Some(matched) = current_match(&cx.workspace.search) {
        let side = match matched.pane {
            SearchPane::Old => Some(("old", old)),
            SearchPane::New => Some(("new", new)),
            SearchPane::Source => None,
        };
        if let Some((side_name, file)) = side {
            let total = cx
                .workspace
                .search
                .as_ref()
                .map_or(0, |search| search.matches.len());
            let current = cx
                .workspace
                .search
                .as_ref()
                .map_or(0, |search| search.cursor + 1);
            let info = format!(
                "search {side_name} line {} · {current}/{total}",
                matched.line
            );
            lines.push(Line::from(Span::styled(
                truncate_ellipsis(&info, width),
                theme.fg(theme.palette.match_fg),
            )));
            if let Some(excerpt) = search_excerpt(file, &matched, width, theme) {
                lines.push(excerpt);
            }
        }
    }

    lines.truncate(height);
    let centered = Rect {
        y: area.y + (area.height - lines.len() as u16) / 2,
        height: lines.len() as u16,
        ..area
    };
    frame.render_widget(Paragraph::new(lines).alignment(Alignment::Center), centered);
}

fn detail_line(text: String, theme: &Theme) -> Line<'static> {
    Line::from(Span::styled(text, theme.fg(theme.palette.text_muted)))
}

fn path_line(prefix: &str, path: &RepoPath, width: usize, theme: &Theme) -> Line<'static> {
    let prefix_width = UnicodeWidthStr::width(prefix);
    let path_width = width.saturating_sub(prefix_width);
    let text = if path_width == 0 {
        truncate_ellipsis(prefix, width)
    } else {
        format!("{prefix}{}", elide_path(&path.to_string(), path_width))
    };
    detail_line(text, theme)
}

fn search_excerpt(
    file: &ProjectedFile,
    matched: &SearchMatch,
    width: usize,
    theme: &Theme,
) -> Option<Line<'static>> {
    let line = file
        .canonical_text()
        .lines()
        .nth(matched.line.checked_sub(1)?)?;
    if width == 0 {
        return None;
    }
    let text = line.to_owned();
    let start = matched.start.min(text.len());
    let end = matched.end.min(text.len()).max(start);
    let runs = vec![Run {
        style: theme.fg(theme.palette.text),
        text: text.clone(),
    }];
    let runs = highlight::apply_emphasis(
        &runs,
        &[(start, end)],
        theme.color(theme.palette.match_current_bg),
    );
    let prefix_width = UnicodeWidthStr::width(&text[..start]);
    let matched_width = UnicodeWidthStr::width(&text[start..end]);
    let skip = if prefix_width + matched_width <= width {
        0
    } else {
        prefix_width.saturating_sub(width.saturating_sub(matched_width).max(1) / 2)
    };
    let clipped = highlight::clip_runs(&runs, skip, width);
    if clipped.is_empty() {
        return None;
    }
    Some(Line::from(
        clipped
            .into_iter()
            .map(|run| Span::styled(run.text, run.style))
            .collect::<Vec<_>>(),
    ))
}

fn pane_title(
    side: Side,
    revision: &str,
    moved_path: Option<&RepoPath>,
    merge_base: bool,
    pane_width: u16,
) -> String {
    let label = match side {
        Side::Old => "BASE",
        Side::New => "TARGET",
    };
    let suffix = if merge_base && side == Side::Old {
        " (merge-base)"
    } else {
        ""
    };
    // Leave room for the rounded border corners on a standalone pane. This
    // also keeps the title within the border in shared-divider layouts.
    let available = (pane_width as usize).saturating_sub(2);
    let Some(path) = moved_path else {
        return truncate_ellipsis(&format!(" {label} · {revision}{suffix} "), available);
    };
    let prefix = format!(" {label} · ");
    let suffix = format!("{suffix} ");
    let full = format!("{prefix}{path}{suffix}");
    if UnicodeWidthStr::width(full.as_str()) <= available {
        return full;
    }
    let subject_width = available.saturating_sub(
        UnicodeWidthStr::width(prefix.as_str()) + UnicodeWidthStr::width(suffix.as_str()),
    );
    if subject_width > 0 {
        let title = format!(
            "{prefix}{}{suffix}",
            elide_path(&path.to_string(), subject_width)
        );
        if UnicodeWidthStr::width(title.as_str()) <= available {
            return title;
        }
    }
    middle_ellipsis(&full, available)
}

fn search_pane(side: Side) -> SearchPane {
    match side {
        Side::Old => SearchPane::Old,
        Side::New => SearchPane::New,
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

    let base = line_background(kind, side, theme);

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
    use base::{ItemKind, ProjectedFile, ProjectedItem, SourceSpan, SupportedPath};
    use tui_core::render::theme::{Capability, Flavor};

    fn projected(path: &str, text: &str) -> ProjectedFile {
        let path =
            SupportedPath::new(RepoPath::new(path).expect("valid path")).expect("supported path");
        ProjectedFile::try_new(
            path,
            vec![ProjectedItem {
                stable_key: "item".to_owned(),
                parent_key: None,
                kind: ItemKind::Function,
                name: "item".to_owned(),
                span: SourceSpan::new(0, 0, 0, 0, 0, 0),
                canonical_text: text.to_owned(),
            }],
        )
        .expect("valid fixture")
    }

    fn moved(old: &str, new: &str, old_text: &str, new_text: &str) -> FileDiff {
        FileDiff::Moved {
            old: projected(old, old_text),
            new: projected(new, new_text),
        }
    }

    #[test]
    fn pane_titles_identify_sides_and_fit_narrow_borders() {
        assert_eq!(
            pane_title(Side::Old, "main", None, false, 20),
            " BASE · main "
        );
        assert_eq!(
            pane_title(Side::New, "feature", None, false, 20),
            " TARGET · feature "
        );

        let narrow = pane_title(Side::New, "a-long-feature-branch", None, false, 16);
        assert_eq!(narrow, " TARGET · a-l…");
        assert_eq!(UnicodeWidthStr::width(narrow.as_str()), 14);
        assert_eq!(pane_title(Side::Old, "main", None, false, 2), "");

        assert_eq!(
            pane_title(Side::Old, "main", None, true, 30),
            " BASE · main (merge-base) "
        );
        assert_eq!(
            pane_title(Side::New, "feature", None, true, 30),
            " TARGET · feature ",
            "the target pane is never the merge base"
        );
    }

    #[test]
    fn a_move_pane_title_names_the_path_that_side_compares() {
        let diff = moved("src/old.rs", "dst/new.rs", "a\n", "b\n");
        assert_eq!(
            pane_title(Side::Old, "base", moved_path(Side::Old, &diff), false, 40),
            " BASE · src/old.rs "
        );
        assert_eq!(
            pane_title(Side::New, "target", moved_path(Side::New, &diff), false, 40),
            " TARGET · dst/new.rs "
        );
        assert_eq!(
            pane_title(Side::Old, "base", moved_path(Side::Old, &diff), true, 60),
            " BASE · src/old.rs (merge-base) "
        );
    }

    #[test]
    fn a_narrow_move_pane_title_keeps_the_differing_basename() {
        let diff = moved(
            "src/very/deep/directories/old.rs",
            "target/other/deep/new.rs",
            "a\n",
            "b\n",
        );
        let old = pane_title(Side::Old, "base", moved_path(Side::Old, &diff), false, 28);
        let new = pane_title(Side::New, "target", moved_path(Side::New, &diff), false, 28);
        assert!(old.starts_with(" BASE · "), "{old}");
        assert!(new.starts_with(" TARGET · "), "{new}");
        assert!(
            old.contains("old.rs"),
            "the old basename must survive: {old}"
        );
        assert!(
            new.contains("new.rs"),
            "the new basename must survive: {new}"
        );
        assert_eq!(UnicodeWidthStr::width(old.as_str()), 26);
        assert_eq!(UnicodeWidthStr::width(new.as_str()), 26);

        let wide = pane_title(Side::Old, "base", moved_path(Side::Old, &diff), false, 56);
        assert_eq!(wide, " BASE · src/very/deep/directories/old.rs ");
    }

    #[test]
    fn a_narrow_move_detail_line_keeps_the_path_basename() {
        let theme = Theme::dark();
        let line = path_line(
            "from ",
            &RepoPath::new("src/very/deep/directories/old.rs").unwrap(),
            26,
            &theme,
        );
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.contains("old.rs"), "{text}");
        assert!(UnicodeWidthStr::width(text.as_str()) <= 26);
    }

    #[test]
    fn a_long_search_excerpt_keeps_the_match_visible_and_highlighted() {
        let theme = Theme::dark();
        let source = format!("{}needle tail\n", "x".repeat(80));
        let file = projected("a.rs", &source);
        let matched = SearchMatch {
            pane: SearchPane::Old,
            line: 1,
            start: 80,
            end: 86,
        };
        let line = search_excerpt(&file, &matched, 20, &theme).expect("an excerpt");
        let text: String = line
            .spans
            .iter()
            .map(|span| span.content.as_ref())
            .collect();
        assert!(text.contains("needle"), "{text}");
        assert!(
            line.spans
                .iter()
                .any(|span| { span.style.bg == Some(theme.color(theme.palette.match_current_bg)) }),
            "the matched bytes stay highlighted after clipping"
        );
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
