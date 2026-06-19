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
          sha256 = "sha256-mvUGEOHYJpn3ikC5hckneuGixaC+yGrkMM/liDIDgoU=";
        };
        craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
        source = pkgs.lib.cleanSource ./.;
        commonArguments = {
          src = source;
          strictDeps = true;
          cargoExtraArgs = "--features live-provider";
        };
        cargoArtifacts = craneLib.buildDepsOnly commonArguments;
        agentPackage = craneLib.buildPackage (
          commonArguments
          // {
            inherit cargoArtifacts;
          }
        );
      in
      {
        packages.default = agentPackage;
        packages.agent = agentPackage;

        checks.build = craneLib.cargoBuild (
          commonArguments
          // {
            inherit cargoArtifacts;
          }
        );
        checks.test = craneLib.cargoTest (
          commonArguments
          // {
            inherit cargoArtifacts;
          }
        );

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
