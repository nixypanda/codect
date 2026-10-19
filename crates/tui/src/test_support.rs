//! Shared test builders for the TUI.
//!
//! `#[cfg(test)]` only. The helpers are `pub(crate)` so the migrated app tests
//! and the golden frames construct the same fixtures from one place.

use base::{
    Area, AreaSet, FileDiff, ItemKind, ProjectedFile, ProjectedItem, ProjectionMode, RepoPath,
    Selection, SourceOutline, SourceSpan, SupportedPath, assemble_outline,
};
use engine::CommitStep;
use ratatui::style::{Color, Modifier};

use crate::kernel::app::{App as GenericApp, Msg as GenericMsg, settle, update};
use crate::kernel::cx::Cx;
use crate::kernel::dispatch::apply_request;
use crate::kernel::plugin::{PluginSet, Request};
use crate::kernel::view::view;
use crate::page::{Diff, DiffViewState, Loaded, Show};
use crate::plugin::{DIFF_BODY_FOCUS, SOURCE_FOCUS};
use crate::plugins::legacy::Msg as LegacyMsg;
use crate::plugins::{KernelOptions, Plugins, PluginsMsg, build};
use crate::render::icons::{IconStyle, Icons};
use crate::render::theme::Theme;
use crate::route::{DiffRequest, DiffView, LoadRequest, ShowRequest};
use crate::scope_chooser::Msg as ScopeMsg;
use crate::util::input::{Key, Mouse, MouseKind};

pub(crate) type App = GenericApp<Plugins>;
pub(crate) type Msg = GenericMsg<PluginsMsg>;
pub(crate) type Cmd = crate::plugins::PluginsCmd;

pub(crate) fn item(text: &str) -> ProjectedItem {
    ProjectedItem {
        stable_key: "item".to_owned(),
        parent_key: None,
        kind: ItemKind::Function,
        name: "item".to_owned(),
        span: SourceSpan::new(0, 0, 0, 0, 0, 0),
        canonical_text: text.to_owned(),
    }
}

pub(crate) fn supported(path: &str) -> SupportedPath {
    SupportedPath::new(RepoPath::new(path).expect("valid path")).expect("test path is supported")
}

pub(crate) fn projected_file(path: &str, text: &str) -> ProjectedFile {
    let path = supported(path);
    if text.is_empty() {
        return ProjectedFile::try_new(path, Vec::new()).expect("valid fixture");
    }
    ProjectedFile::try_new(path, vec![item(text)]).expect("valid fixture")
}

pub(crate) fn source_outline(path: &str, source: &str) -> SourceOutline {
    source_outline_with(path, source, source)
}

pub(crate) fn source_outline_with(
    path: &str,
    source: &str,
    projection_text: &str,
) -> SourceOutline {
    let supported = supported(path);
    let span = whole_span(source);
    let items = |text: &str| -> Vec<ProjectedItem> {
        if text.is_empty() {
            Vec::new()
        } else {
            vec![ProjectedItem {
                kind: ItemKind::Module,
                span,
                ..item(text)
            }]
        }
    };
    let outline =
        assemble_outline(&supported, items(projection_text), items(source)).expect("valid fixture");
    SourceOutline {
        outline,
        source: source.to_owned(),
    }
}

pub(crate) fn whole_span(source: &str) -> SourceSpan {
    let end_line = source.matches('\n').count();
    let end_column = source
        .lines()
        .next_back()
        .map_or(0, |line| line.chars().count());
    SourceSpan::new(0, source.len(), 0, 0, end_line, end_column)
}

pub(crate) fn item_at(
    key: &str,
    parent: Option<&str>,
    kind: ItemKind,
    start_line: usize,
    end_line: usize,
) -> ProjectedItem {
    ProjectedItem {
        stable_key: key.to_owned(),
        parent_key: parent.map(str::to_owned),
        kind,
        name: key.to_owned(),
        span: SourceSpan::new(0, 0, start_line, 0, end_line, 0),
        canonical_text: format!("{key};"),
    }
}

pub(crate) fn nested_outline(
    path: &str,
    source: &str,
    container: (usize, usize),
    child: (usize, usize),
    child_retained: bool,
) -> SourceOutline {
    let supported = supported(path);
    let container = item_at(
        "impl Widget",
        None,
        ItemKind::TraitImplementation,
        container.0,
        container.1,
    );
    let child = item_at(
        "impl Widget::size",
        Some("impl Widget"),
        ItemKind::Method,
        child.0,
        child.1,
    );
    let superset = vec![container.clone(), child.clone()];
    let projection = if child_retained {
        superset.clone()
    } else {
        vec![container]
    };
    let outline = assemble_outline(&supported, projection, superset).expect("valid fixture");
    SourceOutline {
        outline,
        source: source.to_owned(),
    }
}

pub(crate) const NESTED_SOURCE: &str = "impl Widget {\n    fn size(&self) -> usize {\n        let hidden = 42;\n        hidden\n    }\n}\n";

pub(crate) const ATTRIBUTED_SOURCE: &str =
    "impl Widget {\n    #[test]\n    fn tested() {\n        let hidden = 42;\n    }\n}\n";

pub(crate) fn attributed_child_outline(path: &str, source: &str) -> SourceOutline {
    nested_outline(path, source, (0, 5), (2, 4), true)
}

pub(crate) fn test_function_outline(path: &str, source: &str) -> SourceOutline {
    let supported = supported(path);
    let function = item_at("fn::decode_source", None, ItemKind::Function, 1, 3);
    let superset = vec![function.clone()];
    let outline = assemble_outline(&supported, superset.clone(), superset).expect("valid fixture");
    SourceOutline {
        outline,
        source: source.to_owned(),
    }
}

pub(crate) fn dropped_container_outline(path: &str, source: &str) -> SourceOutline {
    let supported = supported(path);
    let container = item_at("impl Widget", None, ItemKind::TraitImplementation, 0, 5);
    let child = item_at(
        "impl Widget::size",
        Some("impl Widget"),
        ItemKind::Method,
        1,
        4,
    );
    let superset = vec![container, child];
    let outline = assemble_outline(&supported, Vec::new(), superset).expect("valid fixture");
    SourceOutline {
        outline,
        source: source.to_owned(),
    }
}

pub(crate) const TESTS_FILTER_SOURCE: &str = "\
pub enum Dropped {\n    A,\n}\n\n#[cfg(test)]\nmod tests {\n    use super::*;\n\n    fn helper() {}\n\n    #[test]\n    fn checks() {\n        let hidden = 1;\n    }\n}\n\n#[test]\nfn top_level() {\n    assert!(true);\n}\n";

pub(crate) fn tests_filter_outline() -> SourceOutline {
    let path = supported("a.rs");
    let dropped = item_at("Dropped", None, ItemKind::Type, 0, 2);
    let module = item_at("mod tests", None, ItemKind::Module, 5, 14);
    let helper = item_at(
        "mod tests::helper",
        Some("mod tests"),
        ItemKind::Function,
        8,
        8,
    );
    let checks = item_at(
        "mod tests::checks",
        Some("mod tests"),
        ItemKind::Function,
        11,
        13,
    );
    let top_level = item_at("top_level", None, ItemKind::Function, 17, 19);
    let projection = vec![module.clone(), checks.clone(), top_level.clone()];
    let superset = vec![dropped, module, helper, checks, top_level];
    let outline = assemble_outline(&path, projection, superset).expect("valid fixture");
    SourceOutline {
        outline,
        source: TESTS_FILTER_SOURCE.to_owned(),
    }
}

pub(crate) fn show_request() -> ShowRequest {
    ShowRequest {
        revision: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
    }
}

pub(crate) fn diff_request() -> DiffRequest {
    DiffRequest {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Range,
        merge_base: false,
    }
}

pub(crate) fn commits_request() -> DiffRequest {
    DiffRequest {
        view: DiffView::Commits,
        ..diff_request()
    }
}

pub(crate) fn file_diff(path: &str, old: Option<&str>, new: Option<&str>) -> FileDiff {
    match (old, new) {
        (None, Some(text)) => FileDiff::Added {
            new: projected_file(path, text),
        },
        (Some(text), None) => FileDiff::Deleted {
            old: projected_file(path, text),
        },
        (Some(old_text), Some(new_text)) => FileDiff::Modified {
            old: projected_file(path, old_text),
            new: projected_file(path, new_text),
        },
        (None, None) => panic!("a test diff needs at least one side"),
    }
}

pub(crate) fn base_app(request: LoadRequest) -> App {
    base_app_with(request, Theme::dark(), Icons::new(IconStyle::None))
}

pub(crate) fn base_app_with(request: LoadRequest, theme: Theme, icons: Icons) -> App {
    build(KernelOptions {
        root: "/repo".to_owned(),
        request,
        scope_label: "all".to_owned(),
        width: 100,
        height: 30,
        theme,
        icons,
    })
    .0
}

/// The plugin-set union of pane specs, as the kernel's layout collects it.
pub(crate) fn panes(app: &App) -> Vec<crate::render::layout::PaneSpec> {
    crate::kernel::app::Specs::panes_only(app).panes
}

pub(crate) fn legacy_msg(msg: LegacyMsg) -> Msg {
    Msg::Plugin(PluginsMsg::Legacy(msg))
}

pub(crate) fn scope_msg(msg: ScopeMsg) -> Msg {
    Msg::Plugin(PluginsMsg::Scope(msg))
}

/// Build the kernel's `Cx` for `app` and call `f` with it.
pub(crate) fn with_cx<R>(app: &App, f: impl FnOnce(&Cx) -> R) -> R {
    let specs = crate::kernel::app::collect_specs(app);
    let cx =
        crate::kernel::app::cx_for(&app.kernel, app.kernel.overlay, app.plugins.busy(), &specs);
    f(&cx)
}

/// The palette commands as `(label, hint)`, in palette order.
pub(crate) fn palette_entries(app: &App) -> Vec<(&'static str, &'static str)> {
    with_cx(app, |cx| {
        cx.palette_commands()
            .into_iter()
            .map(|spec| (spec.label, spec.hint))
            .collect()
    })
}

pub(crate) fn step(app: &App, msg: Msg) -> App {
    settle(update(msg, app).0)
}

pub(crate) fn show_app(files: Vec<SourceOutline>) -> App {
    show_app_mode(files, ProjectionMode::Types)
}

pub(crate) fn show_app_mode(files: Vec<SourceOutline>, mode: ProjectionMode) -> App {
    let app = base_app(LoadRequest::Show {
        revision: "HEAD".to_owned(),
        mode,
        selection: Selection::all(),
    });
    let (next, _) = update(
        legacy_msg(LegacyMsg::ShowLoaded {
            request: ShowRequest {
                revision: "HEAD".to_owned(),
                mode,
                selection: Selection::all(),
            },
            result: Ok(files.into()),
        }),
        &app,
    );
    settle(next)
}

pub(crate) fn diff_app(diffs: Vec<FileDiff>) -> App {
    let app = base_app(LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Range,
        merge_base: false,
    });
    let (next, _) = update(
        legacy_msg(LegacyMsg::DiffLoaded {
            request: diff_request(),
            result: Ok(diffs.into()),
        }),
        &app,
    );
    settle(next)
}

pub(crate) fn commit_steps(count: usize) -> Vec<CommitStep> {
    use git::{HashKind, ObjectId};
    (0..count)
        .map(|index| CommitStep {
            parent_id: ObjectId {
                kind: HashKind::Sha1,
                bytes: vec![index as u8; 20],
            },
            commit_id: ObjectId {
                kind: HashKind::Sha1,
                bytes: vec![(index + 1) as u8; 20],
            },
            subject: format!("Commit subject {index}"),
        })
        .collect()
}

pub(crate) fn commits_app(count: usize) -> App {
    let app = base_app(LoadRequest::Diff {
        base: "HEAD~1".to_owned(),
        target: "HEAD".to_owned(),
        mode: ProjectionMode::Types,
        selection: Selection::all(),
        view: DiffView::Commits,
        merge_base: false,
    });
    let (next, _) = update(
        legacy_msg(LegacyMsg::HistoryLoaded {
            request: commits_request(),
            result: Ok((
                commit_steps(count).into(),
                vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into(),
            )),
        }),
        &app,
    );
    settle(next)
}

pub(crate) fn snapshot_range_app() -> App {
    let request = DiffRequest {
        base: ":index".to_owned(),
        target: ":worktree".to_owned(),
        ..diff_request()
    };
    let app = base_app(LoadRequest::Diff {
        base: request.base.clone(),
        target: request.target.clone(),
        mode: request.mode,
        selection: request.selection.clone(),
        view: DiffView::Range,
        merge_base: request.merge_base,
    });
    let (next, _) = update(
        legacy_msg(LegacyMsg::DiffLoaded {
            request,
            result: Ok(vec![file_diff("a.rs", None, Some("a\n"))].into()),
        }),
        &app,
    );
    settle(next)
}

pub(crate) fn merge_base_range_app() -> App {
    let request = DiffRequest {
        base: "main".to_owned(),
        target: "feature".to_owned(),
        merge_base: true,
        ..diff_request()
    };
    let app = base_app(LoadRequest::Diff {
        base: request.base.clone(),
        target: request.target.clone(),
        mode: request.mode,
        selection: request.selection.clone(),
        view: DiffView::Range,
        merge_base: request.merge_base,
    });
    let (next, _) = update(
        legacy_msg(LegacyMsg::DiffLoaded {
            request,
            result: Ok(vec![file_diff("a.rs", Some("a\n"), Some("A\n"))].into()),
        }),
        &app,
    );
    settle(next)
}

pub(crate) fn tail_palette_entries() -> Vec<(&'static str, &'static str)> {
    vec![
        ("Find file", "Ctrl-F"),
        ("Search in view", "/"),
        ("Switch pane", "Tab"),
        ("Widen file tree", "]"),
        ("Narrow file tree", "["),
        ("Reset file tree", "\\"),
        ("Jump to top", "g"),
        ("Jump to bottom", "G"),
        ("Help", "?"),
        ("Quit", "q"),
    ]
}

pub(crate) fn show_ref(app: &App) -> &Show {
    match &app.plugins.legacy.loaded {
        Loaded::Show(show) => show,
        _ => panic!("expected a show"),
    }
}

pub(crate) fn diff_ref(app: &App) -> &Diff {
    match &app.plugins.legacy.loaded {
        Loaded::Diff(diff) => diff,
        _ => panic!("expected a diff"),
    }
}

pub(crate) fn commits_ref(app: &App) -> &crate::component::commit_picker::CommitPicker {
    match &app.plugins.legacy.loaded {
        Loaded::Diff(diff) => match &diff.view {
            DiffViewState::Commits(commits) => &commits.picker,
            _ => panic!("expected commits"),
        },
        _ => panic!("expected a diff"),
    }
}

pub(crate) fn selected(app: &App) -> Option<String> {
    app.kernel
        .workspace
        .selected
        .as_ref()
        .map(ToString::to_string)
}

/// Select through the kernel request path, as the tree and finder do.
pub(crate) fn select_file(app: &mut App, path: RepoPath) {
    apply(app, Request::Select(Some(path)));
}

/// Commit a search through the kernel request path, as the search input does.
pub(crate) fn commit_search(app: &mut App, needle: &str) {
    apply(app, Request::SetSearch(needle.to_owned()));
}

fn apply(app: &mut App, request: Request) {
    let specs = crate::kernel::app::collect_specs(app);
    apply_request(app, request, &mut Vec::new(), 0, &specs);
}

/// Run the plugin `after_batch` hook and return the effects it produced.
pub(crate) fn after_batch_effects(app: &mut App, pending: &[Cmd]) -> Vec<Cmd> {
    let specs = crate::kernel::app::collect_specs(app);
    let outs = {
        let kernel = &app.kernel;
        let overlay = app.kernel.overlay;
        let busy = app.plugins.busy();
        let cx = crate::kernel::app::cx_for(kernel, overlay, busy, &specs);
        app.plugins.after_batch(pending, &cx)
    };
    let mut cmds = Vec::new();
    crate::kernel::dispatch::process_wrapped(app, outs, &mut cmds, 0, &specs);
    cmds
}

pub(crate) fn diff_focus_of(app: &App) -> Option<crate::kernel::plugin::FocusKey> {
    app.kernel.focus
}

pub(crate) fn focus_content(app: &mut App) {
    app.kernel.focus = Some(match &app.plugins.legacy.loaded {
        Loaded::Show(_) => SOURCE_FOCUS,
        Loaded::Diff(_) => DIFF_BODY_FOCUS,
    });
}

pub(crate) fn press(app: &App, keys: &[Key]) -> App {
    let mut current = app.clone();
    for &key in keys {
        let (next, _) = update(Msg::Key(key), &current);
        current = settle(next);
    }
    current
}

pub(crate) fn two_files() -> App {
    show_app(vec![
        source_outline("a.rs", "a\n"),
        source_outline("b.rs", "b\n"),
    ])
}

pub(crate) const TWO_FUNCTION_SOURCE: &str = "fn a() {\n    one\n}\nfn b() {\n    two\n}\n";

pub(crate) fn two_function_outline(path: &str) -> SourceOutline {
    let supported = supported(path);
    let a = item_at("fn::a", None, ItemKind::Function, 0, 2);
    let b = item_at("fn::b", None, ItemKind::Function, 3, 5);
    let superset = vec![a.clone(), b.clone()];
    let outline = assemble_outline(&supported, superset.clone(), superset).expect("valid fixture");
    SourceOutline {
        outline,
        source: TWO_FUNCTION_SOURCE.to_owned(),
    }
}

pub(crate) fn declaration_less(path: &str, source: &str) -> SourceOutline {
    let supported = supported(path);
    let outline = assemble_outline(&supported, Vec::new(), Vec::new()).expect("valid fixture");
    SourceOutline {
        outline,
        source: source.to_owned(),
    }
}

pub(crate) fn area_set() -> AreaSet {
    AreaSet::new([
        Area::new("core", [RepoPath::new("src/core").unwrap()]).unwrap(),
        Area::new("web", [RepoPath::new("src/web").unwrap()]).unwrap(),
    ])
    .unwrap()
}

pub(crate) fn click(column: u16, row: u16) -> Mouse {
    Mouse {
        column,
        row,
        kind: MouseKind::Click,
    }
}

pub(crate) fn scroll(column: u16, row: u16, up: bool) -> Mouse {
    Mouse {
        column,
        row,
        kind: if up {
            MouseKind::ScrollUp
        } else {
            MouseKind::ScrollDown
        },
    }
}

pub(crate) fn render(app: &App, width: u16, height: u16) -> ratatui::buffer::Buffer {
    let backend = ratatui::backend::TestBackend::new(width, height);
    let mut terminal = ratatui::Terminal::new(backend).expect("terminal");
    terminal.draw(|frame| view(app, frame)).expect("draw");
    terminal.backend().buffer().clone()
}

pub(crate) fn buffer_text(buffer: &ratatui::buffer::Buffer) -> String {
    let mut text = String::new();
    for y in 0..buffer.area.height {
        for x in 0..buffer.area.width {
            if let Some(cell) = buffer.cell((x, y)) {
                text.push_str(cell.symbol());
            }
        }
        text.push('\n');
    }
    text
}

pub(crate) fn buffer_row(buffer: &ratatui::buffer::Buffer, y: u16) -> String {
    (0..buffer.area.width)
        .filter_map(|x| buffer.cell((x, y)))
        .map(|cell| cell.symbol())
        .collect()
}

pub(crate) fn buffer_styles(buffer: &ratatui::buffer::Buffer) -> String {
    let mut styles = String::new();
    for y in 0..buffer.area.height {
        styles.push_str(&format!("{y}:"));
        let mut x = 0;
        while x < buffer.area.width {
            let Some(cell) = buffer.cell((x, y)) else {
                break;
            };
            let key = (cell.fg, cell.bg, cell.modifier);
            let mut end = x;
            while end + 1 < buffer.area.width {
                let Some(next) = buffer.cell((end + 1, y)) else {
                    break;
                };
                if (next.fg, next.bg, next.modifier) != key {
                    break;
                }
                end += 1;
            }
            styles.push_str(&format!(
                " {x}-{end} fg={} bg={} mod={}",
                color_token(cell.fg),
                color_token(cell.bg),
                modifier_token(cell.modifier),
            ));
            x = end + 1;
        }
        styles.push('\n');
    }
    styles
}

pub(crate) fn golden_document(
    name: &str,
    width: u16,
    height: u16,
    buffer: &ratatui::buffer::Buffer,
) -> String {
    let mut document = String::new();
    document.push_str(&format!("# {name} {width}x{height}\n"));
    for y in 0..height {
        document.push_str(&buffer_row(buffer, y));
        document.push('\n');
    }
    document.push_str("## styles\n");
    document.push_str(&buffer_styles(buffer));
    document
}

fn color_token(color: Color) -> String {
    match color {
        Color::Reset => "reset".to_owned(),
        Color::Rgb(red, green, blue) => format!("#{red:02x}{green:02x}{blue:02x}"),
        Color::Indexed(index) => format!("idx:{index}"),
        Color::Black => "black".to_owned(),
        Color::Red => "red".to_owned(),
        Color::Green => "green".to_owned(),
        Color::Yellow => "yellow".to_owned(),
        Color::Blue => "blue".to_owned(),
        Color::Magenta => "magenta".to_owned(),
        Color::Cyan => "cyan".to_owned(),
        Color::Gray => "gray".to_owned(),
        Color::DarkGray => "darkgray".to_owned(),
        Color::LightRed => "lightred".to_owned(),
        Color::LightGreen => "lightgreen".to_owned(),
        Color::LightYellow => "lightyellow".to_owned(),
        Color::LightBlue => "lightblue".to_owned(),
        Color::LightMagenta => "lightmagenta".to_owned(),
        Color::LightCyan => "lightcyan".to_owned(),
        Color::White => "white".to_owned(),
    }
}

fn modifier_token(modifier: Modifier) -> String {
    const FLAGS: [(Modifier, &str); 9] = [
        (Modifier::BOLD, "BOLD"),
        (Modifier::DIM, "DIM"),
        (Modifier::ITALIC, "ITALIC"),
        (Modifier::UNDERLINED, "UNDERLINED"),
        (Modifier::SLOW_BLINK, "SLOW_BLINK"),
        (Modifier::RAPID_BLINK, "RAPID_BLINK"),
        (Modifier::REVERSED, "REVERSED"),
        (Modifier::HIDDEN, "HIDDEN"),
        (Modifier::CROSSED_OUT, "CROSSED_OUT"),
    ];
    let present: Vec<&str> = FLAGS
        .iter()
        .filter(|(flag, _)| modifier.contains(*flag))
        .map(|(_, name)| *name)
        .collect();
    if present.is_empty() {
        "-".to_owned()
    } else {
        present.join("|")
    }
}

pub(crate) fn row_with(buffer: &ratatui::buffer::Buffer, needle: &str) -> Option<u16> {
    (0..buffer.area.height).find(|&y| buffer_row(buffer, y).contains(needle))
}

pub(crate) fn cell_x_of(buffer: &ratatui::buffer::Buffer, y: u16, needle: &str) -> Option<u16> {
    let cells: Vec<&str> = (0..buffer.area.width)
        .filter_map(|x| buffer.cell((x, y)))
        .map(|cell| cell.symbol())
        .collect();
    let joined = cells.concat();
    let byte = joined.find(needle)?;
    let mut offset = 0;
    for (index, symbol) in cells.iter().enumerate() {
        if offset == byte {
            return Some(index as u16);
        }
        offset += symbol.len();
    }
    None
}

pub(crate) fn folder_app() -> App {
    show_app(vec![nested_outline(
        "a.rs",
        NESTED_SOURCE,
        (0, 5),
        (1, 4),
        true,
    )])
}

pub(crate) fn assert_view_consistent(app: &App, height: u16) {
    let show = show_ref(app);
    let selected = app.kernel.workspace.selected.as_ref();
    let count = show.line_count();
    assert!(count > 0, "the projection must not be empty");
    assert!(
        (show.body.scroll as usize) < count,
        "scroll {} must be below line_count {}",
        show.body.scroll,
        count
    );
    let max_h = show.max_line_width(selected).saturating_sub(1) as u16;
    assert!(
        show.body.hscroll <= max_h,
        "hscroll {} must be <= max_line_width-1 {}",
        show.body.hscroll,
        max_h
    );
    let viewport = height.saturating_sub(4) as usize;
    assert!(viewport > 0, "the test viewport must be non-empty");
    if let Some(row) = show.cursor_row() {
        assert!(
            row >= show.body.scroll as usize && row < show.body.scroll as usize + viewport,
            "cursor row {row} must be inside [{}..{})",
            show.body.scroll,
            show.body.scroll as usize + viewport
        );
    }
}

pub(crate) fn multi_method_outline(path: &str, methods: usize) -> SourceOutline {
    let supported = supported(path);
    let mut source = String::from("impl Widget {\n");
    for index in 0..methods {
        source.push_str(&format!("    fn m{index}(&self) {{}}\n"));
    }
    source.push_str("}\n");
    let last = methods + 1;
    let container = item_at("impl Widget", None, ItemKind::TraitImplementation, 0, last);
    let superset: Vec<ProjectedItem> = std::iter::once(container)
        .chain((0..methods).map(|index| {
            let line = index + 1;
            item_at(
                &format!("impl Widget::m{index}"),
                Some("impl Widget"),
                ItemKind::Method,
                line,
                line,
            )
        }))
        .collect();
    let outline = assemble_outline(&supported, superset.clone(), superset).expect("valid fixture");
    SourceOutline { outline, source }
}
