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

# Format Rust sources.
format-workspace:
    cargo fmt --all

# Run all workspace checks (TECHNICAL_DESIGN.md 18.1).
check-workspace: check-workspace-format check-workspace-clippy test-workspace build-workspace check-workspace-features

check-workspace-format:
    cargo fmt --all --check

check-workspace-clippy:
    cargo clippy --workspace --all-targets -- -D warnings

# Audit the enabled gix feature set (TECHNICAL_DESIGN.md 4.1).
check-workspace-features:
    cargo tree -e features -p ownai-git

# Run workspace coverage.
coverage-workspace:
    cargo llvm-cov --workspace

# Format Nix sources.
format-nix:
    nix fmt
