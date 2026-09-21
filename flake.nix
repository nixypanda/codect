{
  description = "OwnAI: selectable focused views and diffs of a codebase";

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
        f: nixpkgs.lib.genAttrs systems (system: f (import nixpkgs { inherit system; overlays = [ (import rust-overlay) ]; }));

      rustToolchain =
        pkgs:
        pkgs.rust-bin.stable.latest.default.override {
          extensions = [ "rust-src" ];
        };
    in
    {
      devShells = forAllSystems (pkgs: {
        default = pkgs.mkShell {
          packages = [
            (rustToolchain pkgs)
            pkgs.rust-bin.stable.latest.rust-analyzer
            pkgs.cargo-llvm-cov
            pkgs.cargo-insta
            pkgs.just
          ];

          env = {
            RUST_BACKTRACE = "1";
          };
        };
      });
    };
}
