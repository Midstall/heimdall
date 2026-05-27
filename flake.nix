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
            ...
          }:
          let
            inherit (pkgs) lib;
            craneLib = crane.mkLib pkgs;

            heimdall-logo-pkg = pkgs.callPackage ./pkgs/heimdall-logo { };

            commonArgs = {
              inherit src;
              inherit (flakeverConfig) version;

              pname = "heimdall-eda";
              strictDeps = true;
              cargoExtraArgs = "--package heimdall-eda";
              # Pre-rendered SVGs picked up by heimdall-daemon's build.rs so
              # the favicon and logomark are embedded without committing
              # generated files into the source tree.
              HEIMDALL_LOGO_SVGS = "${heimdall-logo-pkg.passthru.svgs}";
              # Picked up by heimdall_core::VERSION so the binary reports
              # the full flakever version instead of Cargo.toml's `0.1.0`.
              HEIMDALL_FULL_VERSION = flakeverConfig.version;
            };

            cargoArtifacts = craneLib.buildDepsOnly commonArgs;
          in
          {
            _module.args.pkgs = import inputs.nixpkgs {
              inherit system;
              overlays = [
                inputs.aegis.overlays.default
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
              heimdall-eda = pkgs.callPackage ./pkgs/heimdall-eda {
                inherit
                  craneLib
                  commonArgs
                  src
                  cargoArtifacts
                  ;
              };
              heimdall-logo = heimdall-logo-pkg;
            };

            packages.default = pkgs.heimdall-eda;
            packages.heimdall-logo = heimdall-logo-pkg;
            packages.heimdall-logo-svgs = heimdall-logo-pkg.passthru.svgs;
            # Dev shell carries python3 + the heimdall-logo package so a
            # plain `cargo build` from inside `nix develop` can regenerate
            # the SVGs without setting $HEIMDALL_LOGO_SVGS.
            devShells.default = pkgs.heimdall-eda.shell.overrideAttrs (old: {
              nativeBuildInputs = (old.nativeBuildInputs or [ ]) ++ [
                (pkgs.python3.withPackages (_: [ heimdall-logo-pkg ]))
              ];
            });

            checks = {
              inherit (pkgs) heimdall-eda;

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
