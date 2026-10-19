//! Scrolling vocabulary shared by the kernel and the panes.

/// A page-sized navigation the kernel sends to the focused pane.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Nav {
    Top,
    Bottom,
    PageUp,
    PageDown,
}

/// The half-page step for a viewport of `height` rows.
pub fn page_step(height: u16) -> u16 {
    (height.saturating_sub(3) / 2).max(1)
}

/// Apply a scroll navigation to `current`, clamped to `max`.
pub fn scroll_by(nav: Nav, current: u16, step: u16, max: u16) -> u16 {
    match nav {
        Nav::Top => 0,
        Nav::Bottom => max,
        Nav::PageUp => current.saturating_sub(step),
        Nav::PageDown => current.saturating_add(step).min(max),
    }
}

/// The scroll offset that brings `index` into a viewport of `height` starting at
/// `scroll`, or `None` when it is already visible.
pub fn scroll_offset_to(index: usize, scroll: usize, height: usize) -> Option<u16> {
    if index < scroll {
        Some(index as u16)
    } else if height > 0 && index >= scroll + height {
        Some((index + 1 - height) as u16)
    } else {
        None
    }
}
