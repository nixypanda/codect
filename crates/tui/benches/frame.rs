//! Frame-rendering benchmarks for the terminal frontend.
//!
//! Run with `cargo bench -p tui --features bench --bench frame`, or
//! `just bench-tui`. The `bench` feature exposes the `#[doc(hidden)]`
//! `tui::bench` façade so this target can drive the crate's real
//! `view`, highlighter, and diff layout without widening the public API.
//!
//! # What is measured
//!
//! A frame is not one cost. The run loop first handles a message (`update`),
//! which recomputes syntax highlighting and the wrapped diff layout when the
//! selection or size changed, and then draws (`view` through `Terminal::draw`),
//! which builds widgets and lets ratatui diff the surface.
//!
//! - `frame/warm/*` reuses one terminal, so the ratatui buffer diff is small
//!   (an idle or small-scroll frame).
//! - `frame/cold/*` builds a fresh terminal per iteration, so the first draw
//!   writes every cell (the frame after a load or a large jump).
//! - `update/*` measures message handling, where highlighting and diff layout
//!   live. `load_*` and `*_next_file` are the cold half of an interaction.
//! - `parts/*` measures each seam in isolation: `highlight`, `layout_diff`, and
//!   the ratatui surface diff.
//!
//! `frame/warm − parts/buffer_diff` approximates frame assembly, and
//! `update/load_*` plus `parts/*` explains what a cold frame adds. The
//! crossterm backend's escape-sequence writing is deliberately out of scope:
//! `TestBackend` replaces it with an in-memory surface.

use std::fs;
use std::hint::black_box;
use std::path::PathBuf;
use std::time::Duration;

use base::{FileDiff, Language, ProjectedFile};
use criterion::measurement::WallTime;
use criterion::{
    BatchSize, BenchmarkGroup, BenchmarkId, Criterion, Throughput, criterion_group, criterion_main,
};
use tui::bench::{self, Key, Model, Msg};

/// A wide terminal: tree and content side by side.
const WIDE: (u16, u16) = (140, 40);
/// The narrower end of the side-by-side range.
const MEDIUM: (u16, u16) = (100, 30);
/// Single-pane mode, below the side-by-side threshold.
const NARROW: (u16, u16) = (60, 20);
/// Below the minimum size, so the "terminal too small" view draws.
const TINY: (u16, u16) = (30, 6);

// ---------------------------------------------------------------------------
// Fixtures and scenarios
// ---------------------------------------------------------------------------

fn fixture(relative: &str) -> String {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../fixtures")
        .join(relative);
    fs::read_to_string(&path).unwrap_or_else(|error| panic!("read {}: {error}", path.display()))
}

fn rust_source() -> String {
    fixture("rust/structs/input.rs")
}

fn elm_source() -> String {
    fixture("elm/normal-module/input.elm")
}

/// Repeats the Rust fixture until it has at least `lines` lines, so a long file
/// still highlights like real Rust rather than one repeated word.
fn long_rust(lines: usize) -> String {
    let base = rust_source();
    let base_lines = base.lines().count().max(1);
    let mut text = base.repeat(lines.div_ceil(base_lines));
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text
}

/// A `show` projection of a small Elm and Rust file.
fn show_two_files(width: u16, height: u16) -> Model {
    bench::show_model(
        vec![
            bench::projected("src/Main.elm", &elm_source()),
            bench::projected("src/lib.rs", &rust_source()),
        ],
        width,
        height,
    )
}

/// A `show` projection of one long Rust file.
fn show_long_file(width: u16, height: u16) -> Model {
    bench::show_model(
        vec![bench::projected("src/lib.rs", &long_rust(4000))],
        width,
        height,
    )
}

/// A `show` projection with a large tree of many small files.
fn show_large_tree(width: u16, height: u16) -> Model {
    many_files(300, width, height)
}

fn many_files(count: usize, width: u16, height: u16) -> Model {
    let files = (0..count)
        .map(|index| {
            let path = format!("src/file_{index:03}.rs");
            bench::projected(&path, &long_rust(20))
        })
        .collect();
    bench::show_model(files, width, height)
}

/// A single focused diff with several scattered changes.
fn diff_one_large(width: u16, height: u16) -> Model {
    bench::diff_model(vec![large_diff()], width, height)
}

fn large_diff() -> FileDiff {
    let old_lines: Vec<String> = (0..800)
        .map(|index| format!("    field_{index}: u32,"))
        .collect();
    let mut new_lines = old_lines.clone();
    for index in (0..old_lines.len()).step_by(37) {
        new_lines[index] = format!("    field_{index}: u64,");
    }
    let old = old_lines.join("\n") + "\n";
    let new = new_lines.join("\n") + "\n";
    bench::file_diff("src/lib.rs", Some(&old), Some(&new))
}

/// Many focused diffs, so moving the selection recomputes highlight and layout.
fn many_diffs(count: usize) -> Vec<FileDiff> {
    (0..count)
        .map(|index| {
            let old = format!("pub fn item_{index}() -> u32 {{\n    0\n}}\n");
            let new = format!("pub fn item_{index}() -> u64 {{\n    0\n}}\n");
            bench::file_diff(&format!("src/file_{index:03}.rs"), Some(&old), Some(&new))
        })
        .collect()
}

fn diff_many(count: usize, width: u16, height: u16) -> Model {
    bench::diff_model(many_diffs(count), width, height)
}

fn show_files(count: usize) -> Vec<ProjectedFile> {
    (0..count)
        .map(|index| {
            let path = format!("src/file_{index:03}.rs");
            bench::projected(&path, &long_rust(20))
        })
        .collect()
}

fn press(model: &Model, key: Key) -> Model {
    bench::update(Msg::Key(key), model).0
}

fn type_text(model: &Model, text: &str) -> Model {
    text.chars()
        .fold(model.clone(), |next, ch| press(&next, Key::Char(ch)))
}

// ---------------------------------------------------------------------------
// Frame benchmarks
// ---------------------------------------------------------------------------

fn bench_warm(
    group: &mut BenchmarkGroup<'_, WallTime>,
    name: &str,
    model: &Model,
    (width, height): (u16, u16),
) {
    let mut terminal = bench::terminal(width, height);
    group.throughput(Throughput::Elements(u64::from(width) * u64::from(height)));
    group.bench_function(name, |b| {
        b.iter(|| bench::draw(black_box(model), &mut terminal));
    });
}

fn bench_cold(
    group: &mut BenchmarkGroup<'_, WallTime>,
    name: &str,
    model: &Model,
    (width, height): (u16, u16),
) {
    group.throughput(Throughput::Elements(u64::from(width) * u64::from(height)));
    group.bench_function(name, |b| {
        b.iter_batched(
            || bench::terminal(width, height),
            |mut terminal| bench::draw(black_box(model), &mut terminal),
            BatchSize::SmallInput,
        );
    });
}

fn configure(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sample_size(30);
    group.measurement_time(Duration::from_secs(2));
    group.warm_up_time(Duration::from_secs(1));
}

fn frame_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame/warm");
    configure(&mut group);

    bench_warm(
        &mut group,
        "show_two_files",
        &show_two_files(WIDE.0, WIDE.1),
        WIDE,
    );
    bench_warm(
        &mut group,
        "show_long_file",
        &show_long_file(WIDE.0, WIDE.1),
        WIDE,
    );
    bench_warm(
        &mut group,
        "show_large_tree",
        &show_large_tree(WIDE.0, WIDE.1),
        WIDE,
    );
    bench_warm(
        &mut group,
        "diff_single",
        &diff_one_large(WIDE.0, WIDE.1),
        WIDE,
    );

    let mut scrolled = diff_one_large(WIDE.0, WIDE.1);
    scrolled.body_scroll = 300;
    bench_warm(&mut group, "diff_scrolled", &scrolled, WIDE);

    bench_warm(
        &mut group,
        "show_narrow",
        &show_two_files(NARROW.0, NARROW.1),
        NARROW,
    );
    bench_warm(
        &mut group,
        "show_tiny",
        &show_two_files(TINY.0, TINY.1),
        TINY,
    );

    let palette = press(&show_two_files(MEDIUM.0, MEDIUM.1), Key::CtrlP);
    bench_warm(&mut group, "overlay_palette", &palette, MEDIUM);

    let finder = type_text(
        &press(&show_large_tree(MEDIUM.0, MEDIUM.1), Key::CtrlF),
        "file_1",
    );
    bench_warm(&mut group, "overlay_finder", &finder, MEDIUM);

    let help = press(&show_two_files(MEDIUM.0, MEDIUM.1), Key::Char('?'));
    bench_warm(&mut group, "overlay_help", &help, MEDIUM);

    let search_open = type_text(
        &press(&show_long_file(MEDIUM.0, MEDIUM.1), Key::Char('/')),
        "pub",
    );
    bench_warm(&mut group, "search_open", &search_open, MEDIUM);

    let search_committed = press(&search_open, Key::Enter);
    bench_warm(&mut group, "search_committed", &search_committed, MEDIUM);

    group.finish();
}

fn cold_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("frame/cold");
    configure(&mut group);

    bench_cold(
        &mut group,
        "show_two_files",
        &show_two_files(WIDE.0, WIDE.1),
        WIDE,
    );
    bench_cold(
        &mut group,
        "diff_single",
        &diff_one_large(WIDE.0, WIDE.1),
        WIDE,
    );
    bench_cold(
        &mut group,
        "show_large_tree",
        &show_large_tree(WIDE.0, WIDE.1),
        WIDE,
    );

    group.finish();
}

// ---------------------------------------------------------------------------
// update (highlighting and diff layout)
// ---------------------------------------------------------------------------

fn update_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("update");
    configure(&mut group);

    let show = show_large_tree(WIDE.0, WIDE.1);
    group.bench_function("show_next_file", |b| {
        b.iter(|| black_box(bench::update(Msg::Key(Key::Down), black_box(&show))));
    });

    let diff = diff_many(40, WIDE.0, WIDE.1);
    group.bench_function("diff_next_file", |b| {
        b.iter(|| black_box(bench::update(Msg::Key(Key::Down), black_box(&diff))));
    });

    let resize_from = diff_one_large(WIDE.0, WIDE.1);
    group.bench_function("diff_resize", |b| {
        b.iter(|| {
            black_box(bench::update(
                Msg::Resize {
                    width: MEDIUM.0,
                    height: MEDIUM.1,
                },
                black_box(&resize_from),
            ))
        });
    });

    group.bench_function("tick", |b| {
        b.iter(|| black_box(bench::update(Msg::Tick, black_box(&diff))));
    });

    let files_300 = show_files(300);
    let empty = bench::show_model(Vec::new(), WIDE.0, WIDE.1);
    group.bench_function("load_show_300_files", |b| {
        b.iter_batched(
            || files_300.clone(),
            |files| black_box(bench::load_show(files, black_box(&empty))),
            BatchSize::SmallInput,
        );
    });

    let diff_files = many_diffs(40);
    let empty_diff = bench::diff_model(Vec::new(), WIDE.0, WIDE.1);
    group.bench_function("load_diff_40_files", |b| {
        b.iter_batched(
            || diff_files.clone(),
            |diffs| black_box(bench::load_diff(diffs, black_box(&empty_diff))),
            BatchSize::SmallInput,
        );
    });

    group.finish();
}

// ---------------------------------------------------------------------------
// Tree scrolling
// ---------------------------------------------------------------------------

/// Rows to scroll through when measuring a burst of tree navigation.
const SCROLL_SIZES: [usize; 4] = [1, 10, 100, 300];

fn scroll_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("scroll");
    group.sample_size(10);
    group.measurement_time(Duration::from_secs(5));
    group.warm_up_time(Duration::from_secs(1));

    let model = show_large_tree(WIDE.0, WIDE.1);

    for n in SCROLL_SIZES {
        // Pure message handling: how much work `update` does per row as the
        // cursor passes over the tree. Highlighting is deferred to `settle`, so
        // this is now cheap regardless of how many rows are crossed.
        group.bench_with_input(BenchmarkId::new("updates_only", n), &n, |b, &n| {
            b.iter_batched(
                || model.clone(),
                |mut current| {
                    for _ in 0..n {
                        current = bench::update(Msg::Key(Key::Down), &current).0;
                    }
                    black_box(current)
                },
                BatchSize::SmallInput,
            );
        });

        // The new runtime: every row is updated, then one settle and one draw.
        group.bench_with_input(BenchmarkId::new("batched", n), &n, |b, &n| {
            let mut terminal = bench::terminal(WIDE.0, WIDE.1);
            b.iter_batched(
                || model.clone(),
                |mut current| {
                    for _ in 0..n {
                        current = bench::update(Msg::Key(Key::Down), &current).0;
                    }
                    current = bench::settle(current);
                    bench::draw(&current, &mut terminal);
                    black_box(current)
                },
                BatchSize::SmallInput,
            );
        });

        // The previous runtime: settle and draw after every single row. This
        // reproduces the pre-coalescing cost for comparison.
        group.bench_with_input(BenchmarkId::new("per_step", n), &n, |b, &n| {
            let mut terminal = bench::terminal(WIDE.0, WIDE.1);
            b.iter_batched(
                || model.clone(),
                |mut current| {
                    for _ in 0..n {
                        current = bench::update(Msg::Key(Key::Down), &current).0;
                        current = bench::settle(current);
                        bench::draw(&current, &mut terminal);
                    }
                    black_box(current)
                },
                BatchSize::SmallInput,
            );
        });
    }

    group.finish();
}

// ---------------------------------------------------------------------------
// Per-part seams
// ---------------------------------------------------------------------------

fn parts_benchmarks(c: &mut Criterion) {
    let mut group = c.benchmark_group("parts");
    configure(&mut group);
    // A 4000-line highlight is hundreds of milliseconds per iteration, so the
    // group needs a longer window than a frame.
    group.measurement_time(Duration::from_secs(10));
    group.sample_size(20);

    let rust = rust_source();
    let elm = elm_source();
    let rust_long = long_rust(4000);
    group.throughput(Throughput::Elements(rust.lines().count() as u64));
    group.bench_function("highlight_rust", |b| {
        b.iter(|| black_box(bench::highlight(black_box(&rust), Language::Rust)));
    });
    group.throughput(Throughput::Elements(elm.lines().count() as u64));
    group.bench_function("highlight_elm", |b| {
        b.iter(|| black_box(bench::highlight(black_box(&elm), Language::Elm)));
    });
    group.throughput(Throughput::Elements(rust_long.lines().count() as u64));
    group.bench_function("highlight_rust_long", |b| {
        b.iter(|| black_box(bench::highlight(black_box(&rust_long), Language::Rust)));
    });

    let diff = diff_one_large(WIDE.0, WIDE.1);
    group.throughput(Throughput::Elements(800));
    group.bench_function("layout_diff", |b| {
        b.iter(|| black_box(bench::compute_diff_rows(black_box(&diff))));
    });

    let surface = bench::render_to_buffer(&show_two_files(WIDE.0, WIDE.1), WIDE.0, WIDE.1);
    let same = surface.clone();
    group.throughput(Throughput::Elements(u64::from(WIDE.0) * u64::from(WIDE.1)));
    group.bench_function("buffer_diff", |b| {
        b.iter(|| black_box(bench::buffer_diff(black_box(&surface), black_box(&same))));
    });

    group.finish();
}

criterion_group!(
    frame,
    frame_benchmarks,
    cold_benchmarks,
    update_benchmarks,
    scroll_benchmarks,
    parts_benchmarks
);
criterion_main!(frame);
