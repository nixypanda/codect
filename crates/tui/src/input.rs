//! Terminal input values, already translated from backend events.
//!
//! The frontend core never sees a `crossterm` type: the runtime translates an
//! event into one of these values and hands it to [`crate::app::update`].

/// A key the frontend understands, already translated from a terminal event.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Key {
    Char(char),
    Up,
    Down,
    Left,
    Right,
    Tab,
    BackTab,
    Enter,
    Esc,
    Backspace,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    CtrlC,
    CtrlD,
    CtrlF,
    CtrlP,
    CtrlU,
}

/// A mouse event the frontend understands, already translated from a terminal
/// event and normalized to a terminal cell.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mouse {
    /// Zero-based terminal column.
    pub column: u16,
    /// Zero-based terminal row.
    pub row: u16,
    pub kind: MouseKind,
}

/// The mouse interactions the frontend acts on. Motion and drag are dropped
/// during translation, so they never reach the core.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseKind {
    Click,
    ScrollUp,
    ScrollDown,
}
