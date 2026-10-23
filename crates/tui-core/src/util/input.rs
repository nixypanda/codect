//! Terminal input values, already translated from backend events.
//!
//! The frontend core never sees a `crossterm` type: the runtime translates an
//! event into one of these values and hands it to [`crate::app::update`].
//! Motion and drag events are dropped during translation. `tui` and `tui-diff`
//! match on these values in their input handlers.

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
    CtrlJ,
    CtrlK,
    CtrlP,
    CtrlU,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Mouse {
    pub column: u16,
    pub row: u16,
    pub kind: MouseKind,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MouseKind {
    Click,
    ScrollUp,
    ScrollDown,
}
