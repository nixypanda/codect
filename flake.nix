{
  description = "OwnAI: selectable focused views and diffs of a codebase";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    # The local Laya decision server. Laya pins its own nixpkgs revision so its
    # prebuilt torch and binary cache resolve; do not make it follow ours.
    laya = {
      url = "github:NandhaKishorM/laya";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust-overlay,
      laya,
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

      ownaiPackage =
        pkgs:
        let
          toolchain = rustToolchain pkgs;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = toolchain;
            rustc = toolchain;
          };
        in
        rustPlatform.buildRustPackage {
          pname = "ownai";
          version = "0.1.0";

          src = self;
          cargoLock.lockFile = ./Cargo.lock;

          cargoBuildFlags = [ "--package=ownai-cli" ];
          cargoTestFlags = [ "--package=ownai-cli" ];

          # The workspace root is a virtual manifest, so install the binary
          # produced by cargoBuildHook directly instead of using `cargo install`.
          installPhase = ''
            runHook preInstall
            install -Dm755 target/${pkgs.stdenv.hostPlatform.rust.cargoShortTarget}/release/ownai \
              "$out/bin/ownai"
            runHook postInstall
          '';

          nativeCheckInputs = [ pkgs.gitMinimal ];

          meta = {
            description = "Selectable focused views and diffs of a codebase";
            homepage = "https://github.com/nixypanda/ownai";
            license = pkgs.lib.licenses.mit;
            mainProgram = "ownai";
            platforms = pkgs.lib.platforms.unix;
          };
        };
    in
    {
      packages = forAllSystems (pkgs: {
        default = ownaiPackage pkgs;
        ownai = ownaiPackage pkgs;
        # Re-exported from the upstream Laya flake so `nix run .#laya-serve`
        # starts the local decision server this repository's `laya` provider
        # talks to.
        laya-serve = laya.packages.${pkgs.stdenv.hostPlatform.system}.laya-serve;
      });

      apps = forAllSystems (pkgs: {
        default = {
          type = "app";
          program = "${ownaiPackage pkgs}/bin/ownai";
        };
        laya-serve = {
          type = "app";
          program = "${laya.packages.${pkgs.stdenv.hostPlatform.system}.laya-serve}/bin/laya-serve";
        };
      });

      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            (rustToolchain pkgs)
            pkgs.rust-bin.stable.latest.rust-analyzer
            pkgs.cargo-llvm-cov
            pkgs.just
          ];

          env = {
            RUST_BACKTRACE = "1";
          };
        };
      });

      formatter = forAllSystems (pkgs: pkgs.nixfmt-tree);
    };
}
