//! Display-width text helpers. Tabs expand to [`TAB_WIDTH`] columns, wide
//! characters are never split, and combining marks stay with their base, so the
//! view stays aligned in any terminal. `tui` and `tui-diff` use them.

use unicode_width::{UnicodeWidthChar, UnicodeWidthStr};

pub const TAB_WIDTH: usize = 4;

pub fn expand_tabs(line: &str) -> String {
    if line.contains('\t') {
        line.replace('\t', &" ".repeat(TAB_WIDTH))
    } else {
        line.to_owned()
    }
}

pub fn clip_line(line: &str, skip: usize, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let expanded = expand_tabs(line);

    let mut out = String::new();
    let mut column = 0usize;
    let mut taken = 0usize;
    for character in expanded.chars() {
        let cells = UnicodeWidthChar::width(character).unwrap_or(0);
        if cells == 0 {
            if column >= skip {
                out.push(character);
            }
            continue;
        }
        if column + cells <= skip {
            column += cells;
            continue;
        }
        if column < skip {
            column += cells;
            continue;
        }
        if taken + cells > width {
            break;
        }
        out.push(character);
        taken += cells;
        column += cells;
    }
    out
}

pub fn truncate_ellipsis(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    let full = unicode_width::UnicodeWidthStr::width(text);
    if full <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let clipped = clip_line(text, 0, width - 1);
    format!("{clipped}…")
}

/// Clip `text` in the middle, keeping its start and end within `width` columns.
///
/// Tree labels and pane titles use this so a narrow terminal preserves both the
/// beginning and the differing tail of a path.
pub fn middle_ellipsis(text: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(text) <= width {
        return text.to_owned();
    }
    if width == 1 {
        return "…".to_owned();
    }
    let head_width = (width - 1) / 2;
    let tail_width = width - 1 - head_width;
    let head = clip_line(text, 0, head_width);
    let tail = take_last_columns(text, tail_width);
    format!("{head}…{tail}")
}

/// Clip a repository path while keeping its final component intact.
///
/// The directory is shortened first and the result carries an ellipsis before
/// the final component, so a move label or pane title keeps the basename that
/// distinguishes old from new.
pub fn elide_path(path: &str, width: usize) -> String {
    if width == 0 {
        return String::new();
    }
    if UnicodeWidthStr::width(path) <= width {
        return path.to_owned();
    }
    let Some(slash) = path.rfind('/') else {
        return middle_ellipsis(path, width);
    };
    let (directory, name) = path.split_at(slash + 1);
    let name_width = UnicodeWidthStr::width(name);
    if name_width >= width {
        return middle_ellipsis(name, width);
    }
    let prefix_width = width - name_width;
    if prefix_width < 2 {
        return name.to_owned();
    }
    let directory = directory.strip_suffix('/').unwrap_or(directory);
    let head_width = prefix_width.saturating_sub(3);
    if head_width == 0 {
        return format!("…/{name}");
    }
    let head = clip_line(directory, 0, head_width);
    let head = head.trim_end_matches('/');
    if head.is_empty() {
        // A wide leading glyph can fail to fit the one-column head budget.
        // Dropping the empty head keeps the result relative and the basename
        // visible instead of producing a false leading `/`.
        return format!("…/{name}");
    }
    format!("{head}/…/{name}")
}

fn take_last_columns(text: &str, width: usize) -> String {
    let mut remaining = width;
    let mut start = text.len();
    for (index, character) in text.char_indices().rev() {
        let cells = UnicodeWidthChar::width(character).unwrap_or(0);
        if cells > remaining {
            break;
        }
        remaining -= cells;
        start = index;
    }
    text[start..].to_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn middle_ellipsis_keeps_both_ends_and_respects_width() {
        assert_eq!(middle_ellipsis("abcdef", 6), "abcdef");
        assert_eq!(middle_ellipsis("abcdef", 5), "ab…ef");
        assert_eq!(middle_ellipsis("abcdef", 4), "a…ef");
        assert_eq!(middle_ellipsis("abcdef", 1), "…");
        assert_eq!(middle_ellipsis("abcdef", 0), "");
        assert_eq!(
            UnicodeWidthStr::width(middle_ellipsis("日本語です", 5).as_str()),
            5
        );
    }

    #[test]
    fn elide_path_keeps_the_basename_visible() {
        let path = "src/very/deep/directories/old.rs";
        assert_eq!(elide_path(path, 17), "src/very/…/old.rs");
        assert_eq!(elide_path(path, 8), "…/old.rs");
        assert_eq!(
            elide_path("target/other/deep/new.rs", 22),
            "target/other/…/new.rs"
        );
        assert_eq!(elide_path("old.rs", 5), "ol…rs");
        assert_eq!(elide_path("src/old.rs", 10), "src/old.rs");
        assert_eq!(elide_path("src/old.rs", 0), "");
    }

    #[test]
    fn elide_path_keeps_wide_relative_paths_relative() {
        for path in ["日本語/old.rs", "😀/dir/old.rs"] {
            let elided = elide_path(path, 10);
            assert_eq!(elided, "…/old.rs", "{path}");
            assert!(!elided.starts_with('/'), "must stay relative: {elided}");
            assert!(UnicodeWidthStr::width(elided.as_str()) <= 10);
        }

        let cjk = elide_path("日本語/old.rs", 11);
        assert_eq!(cjk, "日/…/old.rs");
        assert!(UnicodeWidthStr::width(cjk.as_str()) <= 11);
    }
}
