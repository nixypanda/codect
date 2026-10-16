// Display-width text helpers. Tabs expand to [`TAB_WIDTH`] columns, wide
// characters are never split, and combining marks stay with their base, so the
// view stays aligned in any terminal.

use unicode_width::UnicodeWidthChar;

pub(crate) const TAB_WIDTH: usize = 4;

pub(crate) fn expand_tabs(line: &str) -> String {
    if line.contains('\t') {
        line.replace('\t', &" ".repeat(TAB_WIDTH))
    } else {
        line.to_owned()
    }
}

pub(crate) fn clip_line(line: &str, skip: usize, width: usize) -> String {
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

pub(crate) fn truncate_ellipsis(text: &str, width: usize) -> String {
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
