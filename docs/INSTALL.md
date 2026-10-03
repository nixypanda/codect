# Install

Codect is a Rust workspace. The reproducible toolchain is provided by Nix
flakes: Rust comes from [`rust-overlay`](https://github.com/oxalica/rust-overlay)
pinned to current stable, and nixpkgs is pinned to the `nixos-26.05` release.

The flake builds native packages for `x86_64` and `aarch64` on both macOS and
Linux.

## Install with Nix

From a checkout:

```sh
nix profile install .
```

From GitHub:

```sh
nix profile install github:nixypanda/codect
```

Nix automatically selects the package matching the current system. To update an
installation made from a checkout, run `nix profile upgrade codect`.

Run it without installing:

```sh
nix run . -- --help
```

## Development shell

```sh
nix develop
```

With [direnv](https://direnv.net/) installed, `direnv allow` activates the same
shell automatically via `.envrc`. The shell provides Rust, rust-analyzer,
`cargo-llvm-cov`, `just`, Neovim, and Git.

## Build from a checkout

Inside the dev shell:

```sh
just build          # cargo build --workspace --release
just test           # cargo test --workspace
just check          # the full required check suite
```

The binary lands at `target/release/codect`.

## Flake outputs

| Output | Provides |
| --- | --- |
| `packages.<system>.default`, `.codect` | The `codect` binary |
| `packages.<system>.codect-nvim` | The Neovim plugin (does **not** bundle the binary) |
| `overlays.default` | `codect` and `codect-nvim` |
| `devShells.<system>.default` | The development shell |
| `apps.<system>.default` | `nix run` entry point |

Pair the Neovim plugin with the binary on `PATH`:

```nix
programs.neovim.plugins = [ pkgs.codect-nvim ];
programs.neovim.extraPackages = [ pkgs.codect ];
```

See [editors/nvim/README.md](../editors/nvim/README.md) for plugin setup.

## Build without the terminal frontend

The interactive terminal browser is a default-on feature. To build the CLI
without it:

```sh
cargo build -p cli --no-default-features
```

`tui` then becomes an unknown command and no terminal dependency is linked.

## Next

- [Using the CLI](CLI.md)
- [Contributing](CONTRIBUTING.md)
