{
  description = "Codect: selectable focused views and diffs of a codebase";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
    }:
    let
      # Single source of truth for the package version. The release workflow
      # checks the pushed tag against this line, so keep it in sync with
      # `[workspace.package] version` in Cargo.toml.
      version = "0.1.0";

      systems = [
        "x86_64-darwin"
        "aarch64-darwin"
        "x86_64-linux"
        "aarch64-linux"
      ];

      forAllSystems =
        f:
        nixpkgs.lib.genAttrs systems (
          system:
          f (
            import nixpkgs {
              inherit system;
              overlays = [ (import rust-overlay) ];
            }
          )
        );

      rustToolchain =
        pkgs:
        pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
        };

      codectPackage =
        pkgs:
        let
          toolchain = rustToolchain pkgs;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        rustPlatform.buildRustPackage {
          pname = "codect";
          inherit version;

          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = [ "--package=cli" ];
          cargoTestFlags = [ "--package=cli" ];

          # The workspace root is a virtual manifest, so install the binary
          # produced by cargoBuildHook directly instead of using `cargo install`.
          installPhase = ''
            runHook preInstall
            install -Dm755 target/${pkgs.stdenv.hostPlatform.rust.cargoShortTarget}/release/codect \
              "$out/bin/codect"
            runHook postInstall
          '';

          nativeCheckInputs = [ pkgs.gitMinimal ];

          meta = {
            description = "Selectable focused views and diffs of a codebase";
            homepage = "https://github.com/nixypanda/codect";
            license = pkgs.lib.licenses.agpl3Plus;
            mainProgram = "codect";
            platforms = pkgs.lib.platforms.unix;
          };
        };

      # The Neovim plugin. `buildVimPlugin` copies the source tree verbatim, so
      # `lua/` and `plugin/` land at the store root. It cannot wrap the binary;
      # consumers pair it with `pkgs.codect` on PATH (or set `vim.g.codect_binary`
      # / `CODECT_BIN`).
      codectNvimPackage =
        pkgs:
        pkgs.vimUtils.buildVimPlugin {
          pname = "codect.nvim";
          inherit version;
          src = ./editors/nvim;
          meta = {
            description = "Codect semantic fold viewer for Neovim";
            homepage = "https://github.com/nixypanda/codect";
            license = pkgs.lib.licenses.agpl3Plus;
          };
        };

      # The DeepSeek Harness (DSH) sidebar plugin. Builds the TypeScript sources
      # into lib/ with esbuild and installs the package directory. Consumers add
      # this to their DSH profile bundles as `@nixypanda/dsh-codect`.
      codectDshPackage =
        pkgs:
        let
          src = ./editors/dsh;
          esbuildBin = pkgs.esbuild;
        in
        pkgs.runCommand "codect-dsh"
          {
            nativeBuildInputs = [
              pkgs.nodejs
              pkgs.esbuild
              pkgs.typescript
            ];
            inherit src;
          }
          ''
            set -e

            # The store source is read-only and build.mjs writes lib/ next to it,
            # so build from a writable copy.
            mkdir -p work
            cp -r --no-preserve=mode,ownership "$src"/. work/
            chmod -R u+w work
            cd work

            # Build the TypeScript sources using esbuild binary
            export ESBUILD_BINARY=${esbuildBin}/bin/esbuild
            node scripts/build.mjs

            # Install the package tree and an npm-style tarball. Consumers pass
            # the tarball to `dsh plugin add` so pnpm resolves the package's
            # runtime dependencies (a bare store path installs as a link: and
            # would skip them).
            mkdir -p "$out/tarball/package"
            cp -r package.json cordis.patch.yml README.md lib "$out"/
            cp -r package.json cordis.patch.yml README.md lib "$out/tarball/package"/
            tar -czf "$out/codect-dsh.tgz" -C "$out/tarball" package

            echo "codect-dsh built successfully"
          '';
    in
    {
      packages = forAllSystems (pkgs: {
        default = codectPackage pkgs;
        codect = codectPackage pkgs;
        codect-nvim = codectNvimPackage pkgs;
        codect-dsh = codectDshPackage pkgs;
      });

      overlays.default = final: _prev: {
        codect = codectPackage final;
        codect-nvim = codectNvimPackage final;
        codect-dsh = codectDshPackage final;
      };

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${codectPackage pkgs}/bin/codect";
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            (rustToolchain pkgs)
            pkgs.rust-bin.stable.latest.rust-analyzer
            pkgs.cargo-llvm-cov
            pkgs.just
            pkgs.neovim
            pkgs.nodejs
            pkgs.git
          ];

          env = {
            RUST_BACKTRACE = "1";
          };
        };
      });

      # Run the plugin's headless suite in a throwaway Git repository so
      # `nix flake check` covers the plugin alongside the workspace.
      checks = forAllSystems (pkgs: {
        codect-nvim =
          pkgs.runCommand "codect-nvim-check"
            {
              nativeBuildInputs = [
                pkgs.neovim
                pkgs.git
                (codectPackage pkgs)
              ];
            }
            ''
              export HOME=$TMPDIR
              mkdir -p work/editors
              cp -r ${./editors/nvim} work/editors/nvim
              chmod -R u+w work/editors/nvim
              # The plugin tests resolve some fixtures relative to the repository
              # root, so the check's work dir needs both `editors/nvim` and
              # `fixtures/` to look like a real checkout.
              cp -r ${./fixtures} work/fixtures
              chmod -R u+w work/fixtures
              cd work
              git init -q
              git config user.email check@example.com
              git config user.name check
              git add -A
              git commit -qm fixture
              export CODECT_BIN=${codectPackage pkgs}/bin/codect
              nvim --headless -u NONE -l editors/nvim/tests/run.lua > log.txt 2>&1 || {
                cat log.txt
                exit 1
              }
              cat log.txt
              touch $out
            '';
        codect-dsh =
          pkgs.runCommand "codect-dsh-check"
            {
              nativeBuildInputs = [
                pkgs.nodejs
                pkgs.git
                (codectPackage pkgs)
              ];
            }
            ''
              export HOME=$TMPDIR
              mkdir -p work/editors work/crates
              cp -r ${./editors/dsh} work/editors/dsh
              chmod -R u+w work/editors/dsh
              # `codect show` resolves `crates/base/src/lib.rs` relative to the
              # repository root, so the check's work dir needs a matching tree.
              cp -r ${./crates/base} work/crates/base
              chmod -R u+w work/crates/base
              cd work
              git init -q
              git config user.email check@example.com
              git config user.name check
              git add -A
              git commit -qm fixture
              export CODECT_BIN=${codectPackage pkgs}/bin/codect
              node --test editors/dsh/test/*.test.js > log.txt 2>&1 || {
                cat log.txt
                exit 1
              }
              cat log.txt
              touch $out
            '';
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
