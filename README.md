# OwnAI

OwnAI provides selectable views of a codebase at different levels of detail.
Focused views remove implementation bodies so that a person or tool can study the
shape of the code without reading how it works.

See [docs/PRODUCT.md](./docs/PRODUCT.md) for product behavior and scope, and
[docs/TECHNICAL_DESIGN.md](./docs/TECHNICAL_DESIGN.md) for the implementation contract.

## Development environment

The toolchain is provided by Nix (flakes). Rust itself comes from
[`rust-overlay`](https://github.com/oxalica/rust-overlay) pinned to current
stable; nixpkgs is pinned to the `nixos-26.05` release.

```sh
nix develop
```

With [direnv](https://direnv.net/) installed, `direnv allow` activates the same
shell automatically via `.envrc`.

## Required checks

Run every required development check with one command:

```sh
just check
```

This is equivalent to:

```text
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace --release
cargo tree -e features -p ownai-git
```

The final `cargo tree` command audits the enabled `gix` features. It must show
nothing beyond the approved feature list in TECHNICAL_DESIGN.md section 4.1 and
their unavoidable transitive implications.

Individual recipes are available as `just build`, `just test`, `just format`,
`just check-workspace-clippy`, and `just check-workspace-features`.

## Usage

```text
ownai show --mode <types|signatures> [--path <PATH> | --area <AREA>]... [REVISION]
ownai diff --mode <types|signatures> [--path <PATH> | --area <AREA>]... <BASE> <TARGET>
```

`--path`/`-p` is repeatable and scopes a command to a file or directory (a
directory includes everything beneath it); a path that names nothing in the
projected revision — or in either side of a diff — is an error. Paths are
relative to the current directory and must stay inside the repository.

`--area`/`-a` is repeatable and selects named path groups defined in
`.ownai.toml` at the repository root. Area paths are relative to the repository
root and are read only when `--area` is used; `--area` and `--path` cannot be
combined.

```toml
[areas]
frontend = ["apps/web", "packages/ui"]
backend  = ["services/api"]
```

Focused diffs intentionally hide implementation-only changes: if a function body
changes while its projected declaration is unchanged, the focused diff shows no
change for that function.

## Performance baseline

Wall-clock measurements for representative synthetic and real repositories are
recorded in [docs/TECHNICAL_DESIGN.md](./docs/TECHNICAL_DESIGN.md) section 17.
They are a baseline for comparison, not a target.
