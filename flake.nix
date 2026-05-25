{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flakever.url = "github:numinit/flakever";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    crane.url = "github:ipetkov/crane";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-parts,
      flakever,
      treefmt-nix,
      crane,
      ...
    }@inputs:
    let
      flakeverConfig = flakever.lib.mkFlakever {
        inherit inputs;

        digits = [
          1
          2
          2
        ];
      };
    in
    flake-parts.lib.mkFlake { inherit inputs; } {
      imports = [
        inputs.flake-parts.flakeModules.easyOverlay
        inputs.treefmt-nix.flakeModule
      ];

      flake.versionTemplate = "1.1pre-<lastModifiedDate>-<rev>";

      systems = [
        "aarch64-linux"
        "x86_64-linux"
      ];

      perSystem =
        {
          system,
          pkgs,
          ...
        }:
        let
          inherit (pkgs) lib;
          craneLib = crane.mkLib pkgs;
        in
        {
          _module.args.pkgs = import inputs.nixpkgs {
            inherit system;
            overlays = [
              self.overlays.default
            ];
          };

          treefmt.programs = {
            nixfmt.enable = true;
            rustfmt.enable = true;
          };

          legacyPackages = pkgs;

          overlayAttrs = {
            flakever = flakeverConfig;
            heimdall-eda = pkgs.callPackage ./pkgs/heimdall-eda { };
          };

          packages.default = pkgs.heimdall-eda;
          devShells.default = pkgs.heimdall-eda.shell;
        };
    };
}
