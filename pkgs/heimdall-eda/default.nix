{
  lib,
  craneLib,
  commonArgs,
  src,
  cargoArtifacts,
}:
craneLib.buildPackage (
  commonArgs
  // {
    inherit cargoArtifacts;

    passthru.shell = craneLib.devShell { };

    meta = {
      description = "Heimdall: post-silicon hardware verification suite";
      homepage = "https://github.com/Midstall/heimdall";
      license = lib.licenses.asl20;
      mainProgram = "heimdall";
      platforms = lib.platforms.linux;
    };
  }
)
