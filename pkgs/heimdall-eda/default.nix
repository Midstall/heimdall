{
  lib,
  craneLib,
  commonArgs,
  src,
  cargoArtifacts,
  python3,
  heimdall-logo,
}:
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    passthru.shell = craneLib.devShell {
      packages = [
        (python3.withPackages (_: [ heimdall-logo ]))
      ];
    };

    meta = {
      description = "Heimdall: post-silicon hardware verification suite";
      homepage = "https://github.com/Midstall/heimdall";
      license = lib.licenses.asl20;
      mainProgram = "heimdall";
      platforms = lib.platforms.unix;
    };
  }
)
