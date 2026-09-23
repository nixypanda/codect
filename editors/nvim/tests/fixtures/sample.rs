//! Fixture for the OwnAI Neovim plugin headless tests.
//!
//! It deliberately exercises every fold rule:
//!   * a struct with fields (types keep it open),
//!   * an enum with variants,
//!   * an inherent impl with methods (Types drops the whole block),
//!   * a trait impl (Types keeps the trait implementation),
//!   * free functions (Types drops them).

/// A widget with a name and a size.
#[derive(Clone, Debug)]
pub struct Widget {
    pub name: String,
    pub size: usize,
}

/// The size class of a widget.
pub enum Kind {
    Small,
    Large,
}

impl Widget {
    /// Build a widget.
    pub fn new(name: String) -> Self {
        Self { name, size: 0 }
    }

    /// The widget's size.
    pub fn size(&self) -> usize {
        self.size
    }
}

impl std::fmt::Display for Widget {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "{}", self.name)
    }
}

/// Build a widget from a name.
pub fn make(name: &str) -> Widget {
    Widget::new(name.to_string())
}
