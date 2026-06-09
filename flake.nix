{
  description = "agent - the LLM-API-call daemon (OpenAI-compatible provider HTTP calls).";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";
    flake-utils.url = "github:numtide/flake-utils";
    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
      fenix,
      crane,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };
        toolchain = fenix.packages.${system}.fromToolchainFile {
          file = ./rust-toolchain.toml;
          sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
        };
      in
      {
        # NOTE: the package/checks crane build is intentionally not wired yet.
        # The agent triad contracts (signal-agent, meta-signal-agent) are pinned
        # by `path` to the `agent-llm-call-rewrite` worktrees during development;
        # crane cannot vendor those out-of-tree path sources reproducibly. Once
        # the operator integrates the contracts to main and Cargo.toml switches
        # the deps to `git`/`branch = main`, wire the standard crane
        # buildPackage / cargoTest / cargoClippy checks here (mirroring the
        # signal-* flakes). For now, build and test with `cargo` in the devShell.
        devShells.default = pkgs.mkShell {
          name = "agent";
          packages = [
            pkgs.jujutsu
            pkgs.pkg-config
            toolchain
          ];
        };

        formatter = pkgs.nixfmt-rfc-style;
      }
    );
}
