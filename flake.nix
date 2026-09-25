{
  description = "aip-rs - Runtime primitives for Google API Improvement Proposals: resource names";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      nixpkgs,
      flake-utils,
      rust-overlay,
      ...
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs {
          inherit system;
          overlays = [ (import rust-overlay) ];
        };
        manifest = (pkgs.lib.importTOML ./Cargo.toml).package;
        rust-toolchain = pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;
      in
      {
        # A library: nothing to build on its own, so only the shell CI and the
        # weekly flake update run in. The crate has no dependencies, so it needs
        # nothing beyond the toolchain.
        devShells.default = pkgs.mkShell {
          inherit (manifest) name;
          packages = [ rust-toolchain ];
        };
      }
    );
}
