{
  lib,
  craneLib,
}:
let
  src = lib.fileset.toSource {
    root = ../..;
    fileset = lib.fileset.unions [
      ../../Cargo.toml
      ../../Cargo.lock
      ../../crates
    ];
  };

  commonArgs = {
    inherit src;
    pname = "heimdall-eda";
    strictDeps = true;
    cargoExtraArgs = "--package heimdall-eda";
  };

  cargoArtifacts = craneLib.buildDepsOnly commonArgs;
in
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    passthru.shell = craneLib.devShell {};
  }
)
