# Codect

Selectable focused views and diffs of a codebase. Codect strips implementation
bodies so a person or a tool can read the shape of the code — declarations,
types, and signatures — without reading how it works. It supports Elm, Haskell,
Python, and Rust.

```sh
codect show --mode signatures --path src/     # outline a revision
codect diff --mode types HEAD~1 HEAD          # focused diff
codect tui show --mode types                  # interactive terminal browser
```

```sh
nix profile install github:nixypanda/codect   # install
nix develop                                    # or work from a checkout
```

## Documentation

| Document | Contents |
| --- | --- |
| [Install](docs/INSTALL.md) | Nix flake, dev shell, optional-TUI build |
| [Using the CLI](docs/CLI.md) | Commands, path/area scoping, focused-diff semantics |
| [Terminal UI](docs/TUI.md) | Views, keybindings, mouse, theming |
| [Neovim plugin](editors/nvim/README.md) | Setup, commands, Diffview integration |
| [Product](docs/PRODUCT.md) | What Codect does, its modes, and its limits |
| [Architecture](docs/ARCHITECTURE.md) | Crate map and end-to-end data flow |
| [Contributing](docs/CONTRIBUTING.md) | Dev environment, checks, tests |
| [License](LICENSE) | AGPL-3.0-or-later |
