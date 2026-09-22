# OwnAI — Command runner
#
# Enter `nix develop` once, then run `just <command>`.

set shell := ["bash", "-euo", "pipefail", "-c"]

default:
    @just --list

# Enter the reproducible development environment.
shell:
    nix develop

# Build all projects.
build: build-workspace

# Run all tests.
test: test-workspace

# Run the terminal frontend frame benchmarks.
bench: bench-tui

# Format all source files in place.
format: format-workspace format-nix

# Check the workspace.
check: check-workspace

# Run the coverage report.
coverage: coverage-workspace

# Build the workspace.
build-workspace:
    cargo build --workspace --release

# Run all tests.
test-workspace:
    cargo test --workspace

# Benchmark one rendered frame and its per-part seams (criterion).
bench-tui:
    cargo bench -p ownai-tui --features bench --bench frame

# Lint the benchmark target, which `clippy --all-targets` skips without the
# feature. Kept in `check-workspace` so the benchmark cannot bit-rot.
check-bench-tui:
    cargo clippy -p ownai-tui --features bench --all-targets -- -D warnings

# Format Rust sources.
format-workspace:
    cargo fmt --all

# Run all workspace checks (TECHNICAL_DESIGN.md 18.1).
check-workspace: check-workspace-format check-workspace-clippy check-bench-tui test-workspace build-workspace check-workspace-features check-workspace-nodefault

check-workspace-format:
    cargo fmt --all --check

check-workspace-clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Audit the enabled gix feature set (TECHNICAL_DESIGN.md 4.1).
check-workspace-features:
    cargo tree -e features -p ownai-git

# The terminal frontend and the remote TypeSafe provider are default-on
# optional features; building without defaults must neither fail nor pull
# ratatui, crossterm, terminal-colorsaurus, or an HTTP client into the graph.
check-workspace-nodefault:
    cargo build -p ownai-cli --no-default-features
    @if cargo tree -p ownai-cli --no-default-features | grep -Eq '(ratatui|crossterm|terminal-colorsaurus|ureq)'; then \
        echo "error: ratatui/crossterm/terminal-colorsaurus/ureq leaked into the no-default-features build" >&2; \
        exit 1; \
    fi

# Run workspace coverage.
coverage-workspace:
    cargo llvm-cov --workspace

# Format Nix sources.
format-nix:
    nix fmt
