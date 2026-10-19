//! Help rows: the data a plugin contributes to the help popup.
//!
//! Plugins build rows with these types; the help plugin in `tui` renders them.

#![allow(dead_code)]
/// A help section: a title and its sort rank.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HelpSection {
    pub title: &'static str,
    pub rank: u16,
}

pub const MOVE: HelpSection = HelpSection {
    title: "Move",
    rank: 10,
};
pub const FIND: HelpSection = HelpSection {
    title: "Find",
    rank: 30,
};
pub const VIEW: HelpSection = HelpSection {
    title: "View",
    rank: 40,
};
pub const MOUSE: HelpSection = HelpSection {
    title: "Mouse",
    rank: 50,
};

/// One help row: its section, the keys, the text, and its rank within the
/// section.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HelpEntry {
    pub section: HelpSection,
    pub keys: &'static str,
    pub text: &'static str,
    pub rank: u16,
}
