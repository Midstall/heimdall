{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs";
    flake-parts.url = "github:hercules-ci/flake-parts";
    flakever.url = "github:numinit/flakever";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    crane.url = "github:ipetkov/crane";
    advisory-db = {
      url = "github:rustsec/advisory-db";
      flake = false;
    };
    aegis.url = "github:Midstall/aegis";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-parts,
      flakever,
      treefmt-nix,
      crane,
      advisory-db,
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

      inherit (nixpkgs) lib;

      src = lib.fileset.toSource {
        root = ./.;
        fileset = lib.fileset.unions [
          ./Cargo.toml
          ./Cargo.lock
          ./.cargo/audit.toml
          ./deny.toml
          ./crates
          ./testdata
        ];
      };
    in
    flake-parts.lib.mkFlake { inherit inputs; } (
      { self, ... }:
      {
        imports = [
          inputs.flake-parts.flakeModules.easyOverlay
          inputs.treefmt-nix.flakeModule
        ];

        flake.versionTemplate =
          let
            cargoVersion =
              (import "${crane}/lib/crateNameFromCargoToml.nix" {
                inherit lib;
                internalCrateNameFromCargoToml = import "${inputs.crane}/lib/internalCrateNameFromCargoToml.nix" {
                  inherit lib;
                };
              } { inherit src; }).version;
          in
          "${cargoVersion}pre-<lastModifiedDate>-<rev>";

        flake.nixosModules = {
          default = self.nixosModules.heimdall;
          heimdall = import ./nix/modules/nixos.nix { inherit self; };
        };

        flake.darwinModules = {
          default = self.darwinModules.heimdall;
          heimdall = import ./nix/modules/darwin.nix { inherit self; };
        };

        flake.homeManagerModules = {
          default = self.homeManagerModules.heimdall;
          heimdall = import ./nix/modules/home-manager.nix { inherit self; };
        };

        systems = [
          "aarch64-linux"
          "x86_64-linux"
          "aarch64-darwin"
        ];

        perSystem =
          {
            system,
            pkgs,
            final,
            ...
          }:
          let
            inherit (pkgs) lib;
            craneLib = crane.mkLib pkgs;

            commonArgs = {
              inherit src;
              inherit (flakeverConfig) version;

              pname = "heimdall-eda";
              strictDeps = true;
              cargoExtraArgs = "--package heimdall-eda";
              HEIMDALL_LOGO_SVGS = "${final.heimdall-logo.passthru.svgs}";
              HEIMDALL_FULL_VERSION = flakeverConfig.version;
            };

            cargoArtifacts = craneLib.buildDepsOnly commonArgs;

            testRuntimeInputs = [
              pkgs.llvm
              pkgs.ngspice
            ]
            ++ lib.optionals pkgs.stdenv.hostPlatform.isLinux [
              pkgs.spike
            ];
          in
          {
            _module.args.pkgs = import inputs.nixpkgs {
              inherit system;
              overlays = [
                inputs.aegis.overlays.default
                self.overlays.default
              ];
            };

            treefmt = {
              programs = {
                nixfmt.enable = true;
                rustfmt.enable = true;
                taplo.enable = true;
                ruff-format.enable = true;
                yamlfmt.enable = true;
                prettier.enable = true;
              };
              settings.formatter.prettier.includes = [
                "*.ts"
                "*.css"
                "*.md"
                "*.json"
              ];
              settings.global.excludes = [
                "Cargo.lock"
                "*.lock"
                "testdata/**"
                "crates/heimdall-web/templates/*.html"
                "**/*.S"
                "**/*.sp"
                "**/*.elf"
                "**/LICENSE"
              ];
            };

            legacyPackages = pkgs;

            overlayAttrs = {
              flakever = flakeverConfig;
              heimdall-eda = pkgs.callPackage ./pkgs/heimdall-eda {
                inherit
                  craneLib
                  commonArgs
                  src
                  cargoArtifacts
                  ;
              };
              heimdall-logo = pkgs.callPackage ./pkgs/heimdall-logo { };
            };

            packages.default = pkgs.heimdall-eda;
            packages.heimdall-logo = pkgs.heimdall-logo;
            packages.heimdall-logo-svgs = pkgs.heimdall-logo.passthru.svgs;
            devShells.default = pkgs.heimdall-eda.shell;

            checks = {
              inherit (pkgs) heimdall-eda heimdall-logo;

              workspace-clippy = craneLib.cargoClippy (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  cargoClippyExtraArgs = "--workspace --all-targets -- --deny warnings";
                }
              );

              workspace-doctest = craneLib.cargoTest (
                commonArgs
                // {
                  inherit cargoArtifacts;

                  doCheck = true;

                  cargoTestExtraArgs = "--workspace --doc";
                  nativeBuildInputs = (commonArgs.nativeBuildInputs or [ ]) ++ testRuntimeInputs;
                }
              );

              workspace-test = craneLib.cargoNextest (
                commonArgs
                // {
                  inherit cargoArtifacts;

                  doCheck = true;

                  partitions = 1;
                  partitionType = "count";
                  cargoNextestExtraArgs = "--workspace --all-targets";
                  cargoNextestPartitionsExtraArgs = "--no-tests=pass";
                  nativeBuildInputs = (commonArgs.nativeBuildInputs or [ ]) ++ testRuntimeInputs;
                }
              );

              workspace-audit = craneLib.cargoAudit (
                commonArgs
                // {
                  inherit advisory-db;
                }
              );

              workspace-deny = craneLib.cargoDeny (builtins.removeAttrs commonArgs [ "cargoExtraArgs" ]);

              workspace-shear = craneLib.mkCargoDerivation (
                commonArgs
                // {
                  inherit cargoArtifacts;
                  pname = "heimdall-shear";
                  buildPhaseCargoCommand = "cargo shear --frozen";
                  nativeBuildInputs = (commonArgs.nativeBuildInputs or [ ]) ++ [ pkgs.cargo-shear ];
                }
              );
            }
            //
              # NixOS VM tests, namespaced under `nixos-` so they sort apart
              # from the cargo-driven checks. Linux-only because the NixOS
              # test driver needs QEMU and infra that don't exist on Darwin.
              lib.optionalAttrs pkgs.stdenv.hostPlatform.isLinux (
                lib.mapAttrs' (name: test: lib.nameValuePair "nixos-${name}" test) (
                  import ./nix/tests { inherit self pkgs; }
                )
              );
          };
      }
    );
}
