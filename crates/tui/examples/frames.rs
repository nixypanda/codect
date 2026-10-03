//! Render representative terminal-frontend frames to ANSI for `docs/showcase`.
//!
//! Build with the `bench` feature (which exposes the `bench` façade) and run
//! from the repository root:
//!
//! ```sh
//! cargo run -q -p tui --features bench --example frames -- --out target/showcase-frames
//! ```
//!
//! `scripts/showcase_tui.py` turns the `.ansi` files into `docs/showcase/tui-*.png`.
//! Frames are built from this repository, so they reflect the tree at generation
//! time.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use base::{ProjectionMode, Selection};
use engine::Engine;
use ratatui::buffer::Buffer;
use ratatui::style::{Color, Modifier};
use tui::bench::{
    Key, Model, Msg, diff_model, history_model, render_to_buffer, settle, show_model, update,
};

/// The pinned range for the diff frames, matching the CLI showcase (PR #16).
const RANGE_BASE: &str = "4e09c9c^";
const RANGE_TARGET: &str = "4e09c9c";
/// A pinned, stable first-parent range for the commits picker.
const COMMITS_BASE: &str = "4e09c9c";
const COMMITS_TARGET: &str = "19decfa";

fn main() -> ExitCode {
    let args = match Args::parse() {
        Ok(args) => args,
        Err(error) => {
            eprintln!("frames: {error}");
            return ExitCode::FAILURE;
        }
    };
    if let Err(error) = run(&args) {
        eprintln!("frames: {error}");
        return ExitCode::FAILURE;
    }
    ExitCode::SUCCESS
}

fn run(args: &Args) -> Result<(), Box<dyn std::error::Error>> {
    fs::create_dir_all(&args.out)?;
    let engine = Engine::discover(Path::new("."))?;
    let selection = Selection::all();
    let (width, height) = (args.width, args.height);
    // The bench façade labels the header `/repo`; use the real repository name.
    let root = engine
        .root()
        .canonicalize()
        .ok()
        .or_else(|| std::env::current_dir().ok())
        .and_then(|dir| {
            dir.file_name()
                .map(|name| name.to_string_lossy().into_owned())
        })
        .unwrap_or_else(|| "repo".to_owned());

    let files = engine.show("HEAD", ProjectionMode::Types, &selection)?;
    let mut show = show_model(files, width, height);
    show.root = root.clone();
    render("tui-show", &show, &args.out, width, height)?;

    // The command palette is a global overlay; `update` opens it purely.
    let (palette, _) = update(Msg::Key(Key::CtrlP), &show);
    render("tui-overlay", &settle(palette), &args.out, width, height)?;

    let diffs = engine.diff(RANGE_BASE, RANGE_TARGET, ProjectionMode::Types, &selection)?;
    let mut range = diff_model(diffs, width, height);
    range.root = root.clone();
    render("tui-diff-range", &range, &args.out, width, height)?;

    let steps = engine.first_parent_steps(COMMITS_BASE, COMMITS_TARGET)?;
    let first = match steps.first() {
        Some(step) => engine.diff(
            &step.parent_id.to_string(),
            &step.commit_id.to_string(),
            ProjectionMode::Types,
            &selection,
        )?,
        None => Vec::new(),
    };
    let mut commits = history_model(steps, first, width, height);
    commits.root = root;
    render("tui-diff-commits", &commits, &args.out, width, height)?;

    println!("wrote frames to {}", args.out.display());
    Ok(())
}

/// Renders `model` and writes `<out>/<name>.ansi`.
fn render(name: &str, model: &Model, out: &Path, width: u16, height: u16) -> io::Result<()> {
    let buffer = render_to_buffer(model, width, height);
    fs::write(out.join(format!("{name}.ansi")), buffer_to_ansi(&buffer))
}

/// Serializes a composed frame to ANSI SGR, one line per buffer row.
///
/// Ratatui already holds the final grid, so this is a cell walk, not a terminal
/// emulator. A wide glyph's continuation cell has an empty symbol and is
/// skipped, which keeps the two-column glyph aligned.
fn buffer_to_ansi(buffer: &Buffer) -> String {
    let width = buffer.area.width as usize;
    if width == 0 {
        return String::new();
    }
    let mut output = String::new();
    for row in buffer.content.chunks(width) {
        let mut last = String::new();
        for cell in row {
            let symbol = cell.symbol();
            if symbol.is_empty() {
                continue;
            }
            let style = sgr(cell.fg, cell.bg, cell.modifier);
            if style != last {
                output.push_str(&style);
                last = style;
            }
            output.push_str(symbol);
        }
        output.push_str("\x1b[0m\n");
    }
    output
}

fn sgr(fg: Color, bg: Color, modifier: Modifier) -> String {
    let mut codes = vec!["0".to_owned()];
    for (flag, code) in [
        (Modifier::BOLD, "1"),
        (Modifier::DIM, "2"),
        (Modifier::ITALIC, "3"),
        (Modifier::UNDERLINED, "4"),
        (Modifier::REVERSED, "7"),
        (Modifier::HIDDEN, "8"),
        (Modifier::CROSSED_OUT, "9"),
    ] {
        if modifier.contains(flag) {
            codes.push(code.to_owned());
        }
    }
    if let Some(code) = color_code(fg, false) {
        codes.push(code);
    }
    if let Some(code) = color_code(bg, true) {
        codes.push(code);
    }
    format!("\x1b[{}m", codes.join(";"))
}

fn color_code(color: Color, background: bool) -> Option<String> {
    let normal = if background { 40 } else { 30 };
    let bright = if background { 100 } else { 90 };
    Some(match color {
        Color::Reset => return None,
        Color::Black => normal.to_string(),
        Color::Red => (normal + 1).to_string(),
        Color::Green => (normal + 2).to_string(),
        Color::Yellow => (normal + 3).to_string(),
        Color::Blue => (normal + 4).to_string(),
        Color::Magenta => (normal + 5).to_string(),
        Color::Cyan => (normal + 6).to_string(),
        Color::Gray => (normal + 7).to_string(),
        Color::DarkGray => bright.to_string(),
        Color::LightRed => (bright + 1).to_string(),
        Color::LightGreen => (bright + 2).to_string(),
        Color::LightYellow => (bright + 3).to_string(),
        Color::LightBlue => (bright + 4).to_string(),
        Color::LightMagenta => (bright + 5).to_string(),
        Color::LightCyan => (bright + 6).to_string(),
        Color::White => (bright + 7).to_string(),
        Color::Rgb(r, g, b) => {
            let base = if background { 48 } else { 38 };
            format!("{base};2;{r};{g};{b}")
        }
        Color::Indexed(n) => {
            let base = if background { 48 } else { 38 };
            format!("{base};5;{n}")
        }
    })
}

struct Args {
    out: PathBuf,
    width: u16,
    height: u16,
}

impl Args {
    fn parse() -> Result<Self, String> {
        let mut out = PathBuf::from("target/showcase-frames");
        let mut width = 132;
        let mut height = 40;
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--out" => out = PathBuf::from(args.next().ok_or("--out needs a value")?),
                "--width" => {
                    width = args
                        .next()
                        .ok_or("--width needs a value")?
                        .parse()
                        .map_err(|_| "--width must be a number")?;
                }
                "--height" => {
                    height = args
                        .next()
                        .ok_or("--height needs a value")?
                        .parse()
                        .map_err(|_| "--height must be a number")?;
                }
                other => return Err(format!("unknown argument: {other}")),
            }
        }
        Ok(Self { out, width, height })
    }
}
