//! The grammar-agnostic layout document shared by the language adapters.
//!
//! A [`Doc`] is a small structural pretty-printer for the handful of declaration
//! shapes the projectors emit; it is not a general source formatter. `Indent` is
//! relative, so the same document renders at any nesting depth and a nested
//! item's own fragment matches the text embedded in its ancestor.
//!
//! [`Doc::Group`] is the only width-sensitive construct: it renders flat (soft
//! breaks as spaces) when the flat form fits within [`LINE_WIDTH`], and broken
//! (soft breaks as newlines) otherwise. Terminal width is never consulted, so
//! projected output stays deterministic.

use unicode_width::UnicodeWidthStr;

use crate::render::LINE_WIDTH;

/// A small layout document, rendered by [`render`].
#[derive(Clone, Debug)]
pub enum Doc {
    Text(String),
    Line,
    /// A space when flat, a line break when broken.
    SoftLine,
    /// Nothing when flat, a line break when broken.
    SoftNil,
    /// Text emitted only when the enclosing group is broken, used for the
    /// trailing comma that keeps an appended list item on a single diff line.
    Broken(&'static str),
    Indent(Box<Doc>),
    Group(Box<Doc>),
    Concat(Vec<Doc>),
}

impl Doc {
    /// Renders the document at the top level (nesting depth zero).
    pub fn render(&self) -> String {
        render(self, 0)
    }
}

struct Output {
    text: String,
    depth: usize,
    at_line_start: bool,
    column: usize,
}

impl Output {
    fn write(&mut self, value: &str) {
        if self.at_line_start {
            for _ in 0..self.depth {
                self.text.push_str("    ");
            }
            self.column = self.depth * 4;
            self.at_line_start = false;
        }
        self.text.push_str(value);
        self.column += UnicodeWidthStr::width(value);
    }

    fn line(&mut self) {
        self.text.push('\n');
        self.at_line_start = true;
        self.column = 0;
    }

    fn column_now(&self) -> usize {
        if self.at_line_start {
            self.depth * 4
        } else {
            self.column
        }
    }
}

/// Renders a document at a nesting depth of four spaces per level.
pub fn render(doc: &Doc, depth: usize) -> String {
    let mut output = Output {
        text: String::new(),
        depth,
        at_line_start: true,
        column: 0,
    };
    render_into(doc, &mut output, false);
    output.text
}

fn render_into(doc: &Doc, output: &mut Output, flat: bool) {
    match doc {
        Doc::Text(value) => output.write(value),
        Doc::Line => output.line(),
        Doc::SoftLine => {
            if flat {
                output.write(" ");
            } else {
                output.line();
            }
        }
        Doc::SoftNil => {
            if !flat {
                output.line();
            }
        }
        Doc::Broken(value) => {
            if !flat {
                output.write(value);
            }
        }
        Doc::Indent(inner) => {
            output.depth += 1;
            render_into(inner, output, flat);
            output.depth -= 1;
        }
        Doc::Group(inner) => {
            let flat_here = flat
                || flat_width(inner).is_some_and(|width| output.column_now() + width <= LINE_WIDTH);
            render_into(inner, output, flat_here);
        }
        Doc::Concat(parts) => {
            for part in parts {
                render_into(part, output, flat);
            }
        }
    }
}

// The display width of `doc` rendered flat, or `None` when it contains a hard
// [`Doc::Line`] and can therefore never be flat.
fn flat_width(doc: &Doc) -> Option<usize> {
    match doc {
        Doc::Text(value) => Some(UnicodeWidthStr::width(value.as_str())),
        Doc::Line => None,
        Doc::SoftLine => Some(1),
        Doc::SoftNil | Doc::Broken(_) => Some(0),
        Doc::Indent(inner) | Doc::Group(inner) => flat_width(inner),
        Doc::Concat(parts) => {
            let mut total = 0;
            for part in parts {
                total += flat_width(part)?;
            }
            Some(total)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Doc, render};

    // A comma-separated bracketed list in the shape the adapters build.
    fn list(items: &[String]) -> Doc {
        let mut inner = vec![Doc::SoftNil];
        for (index, item) in items.iter().enumerate() {
            if index > 0 {
                inner.push(Doc::Text(",".to_owned()));
                inner.push(Doc::SoftLine);
            }
            inner.push(Doc::Text(item.clone()));
        }
        inner.push(Doc::Broken(","));
        Doc::Group(Box::new(Doc::Concat(vec![
            Doc::Text("(".to_owned()),
            Doc::Indent(Box::new(Doc::Concat(inner))),
            Doc::SoftNil,
            Doc::Text(")".to_owned()),
        ])))
    }

    #[test]
    fn group_stays_inline_when_it_fits_the_budget() {
        let items = vec!["a: int".to_owned(), "b: int".to_owned()];
        assert_eq!(render(&list(&items), 0), "(a: int, b: int)");
    }

    #[test]
    fn group_breaks_one_item_per_line_over_the_budget() {
        let first = "x".repeat(40);
        let second = "y".repeat(40);
        let items = vec![first.clone(), second.clone()];
        assert_eq!(
            render(&list(&items), 0),
            format!("(\n    {first},\n    {second},\n)")
        );
    }

    #[test]
    fn a_broken_list_indents_relative_to_its_depth() {
        let first = "x".repeat(40);
        let second = "y".repeat(40);
        let items = vec![first.clone(), second.clone()];
        assert_eq!(
            render(&list(&items), 2),
            format!("        (\n            {first},\n            {second},\n        )")
        );
    }

    #[test]
    fn flat_width_counts_display_columns_not_bytes() {
        // Each CJK character is two columns wide. Forty of them overflow the
        // budget even though the byte length is not what decides.
        let wide = "名".repeat(40);
        let items = vec![wide.clone(), wide.clone()];
        assert!(
            render(&list(&items), 0).starts_with("(\n"),
            "expected a broken list"
        );
    }

    #[test]
    fn document_method_renders_at_depth_zero() {
        let doc = list(&["a".to_owned()]);
        assert_eq!(doc.render(), render(&doc, 0));
    }
}
