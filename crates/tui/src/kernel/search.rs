//! The search model: the needle, its matches, and the cursor.
//!
//! Matches are collected per pane and line. `SearchPane` is the temporary pane
//! name that task 4.x replaces with a `PaneId`.

/// The committed search: the needle, its matches in view order, and the cursor
/// that names the current match.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Search {
    pub needle: String,
    pub matches: Vec<SearchMatch>,
    pub cursor: usize,
}

/// One search hit: the pane it belongs to and its byte range on a 1-based line.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SearchMatch {
    pub pane: SearchPane,
    pub line: usize,
    pub start: usize,
    pub end: usize,
}

/// The pane a search match belongs to. Temporary until task 4.x replaces it
/// with a `PaneId`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum SearchPane {
    Source,
    Old,
    New,
}

/// Collect the case-insensitive, non-overlapping matches of `needle_lower` in
/// `text`, tagging each with `pane`. Lines are 1-based; `end` is the byte offset
/// of the character after the match, or the line length at the line's end.
pub(crate) fn collect_matches(
    text: &str,
    needle_lower: &str,
    pane: SearchPane,
    out: &mut Vec<SearchMatch>,
) {
    let needle: Vec<char> = needle_lower.chars().collect();
    if needle.is_empty() {
        return;
    }
    for (index, line) in text.lines().enumerate() {
        let hay: Vec<(usize, char)> = line.char_indices().collect();
        let mut i = 0;
        while i + needle.len() <= hay.len() {
            let matched = (0..needle.len())
                .all(|k| hay[i + k].1.to_lowercase().next().unwrap_or(hay[i + k].1) == needle[k]);
            if matched {
                let start = hay[i].0;
                let end = hay
                    .get(i + needle.len())
                    .map_or(line.len(), |&(byte, _)| byte);
                out.push(SearchMatch {
                    pane,
                    line: index + 1,
                    start,
                    end,
                });
                i += needle.len();
            } else {
                i += 1;
            }
        }
    }
}

/// The search ranges on one line and pane, tagged with whether each is the
/// global cursor's match.
pub(crate) fn search_ranges(
    search: Option<&Search>,
    pane: SearchPane,
    line: usize,
) -> Vec<(usize, usize, bool)> {
    let Some(search) = search else {
        return Vec::new();
    };
    search
        .matches
        .iter()
        .enumerate()
        .filter(|(_, matched)| matched.pane == pane && matched.line == line)
        .map(|(index, matched)| (matched.start, matched.end, index == search.cursor))
        .collect()
}

/// Advance the search cursor with wrap-around and return the new current match.
pub(crate) fn advance_search(search: &mut Option<Search>, forward: bool) -> Option<SearchMatch> {
    let search = search.as_mut()?;
    if search.matches.is_empty() {
        return None;
    }
    let len = search.matches.len();
    search.cursor = if forward {
        (search.cursor + 1) % len
    } else {
        (search.cursor + len - 1) % len
    };
    search.matches.get(search.cursor).cloned()
}

/// The match the search cursor currently names, if any.
pub(crate) fn current_match(search: &Option<Search>) -> Option<SearchMatch> {
    let search = search.as_ref()?;
    search.matches.get(search.cursor).cloned()
}
