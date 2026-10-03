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
          version = "0.1.0";

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
          version = "0.1.0";
          src = ./editors/nvim;
          meta = {
            description = "Codect semantic fold viewer for Neovim";
            homepage = "https://github.com/nixypanda/codect";
            license = pkgs.lib.licenses.agpl3Plus;
          };
        };
    in
    {
      packages = forAllSystems (pkgs: {
        default = codectPackage pkgs;
        codect = codectPackage pkgs;
        codect-nvim = codectNvimPackage pkgs;
      });

      overlays.default = final: _prev: {
        codect = codectPackage final;
        codect-nvim = codectNvimPackage final;
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
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
