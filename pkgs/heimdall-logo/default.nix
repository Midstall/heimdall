{
  flakever,
  lib,
  stdenvNoCC,
  python3,
  python3Packages,
}:
python3Packages.buildPythonPackage (finalAttrs: {
  pname = "heimdall-logo";
  inherit (flakever) version;
  format = "pyproject";

  src = ./.;

  build-system = [ python3Packages.setuptools ];
  propagatedBuildInputs = [ python3Packages.fontforge ];

  pythonImportsCheck = [ "heimdall_logo" ];

  passthru.svgs = stdenvNoCC.mkDerivation {
    pname = "heimdall-logo-svgs";
    inherit (flakever) version;

    dontUnpack = true;

    nativeBuildInputs = [ (python3.withPackages (_: [ finalAttrs.finalPackage ])) ];

    buildPhase = ''
      mkdir -p $out
      heimdall-logo logomark --output $out/heimdall-logomark.svg
      heimdall-logo logomark --output $out/heimdall-logomark-darkbg.svg \
        --background "#1a1b26"
      heimdall-logo favicon  --output $out/heimdall-favicon.svg
    '';

    dontInstall = true;

    meta = with lib; {
      description = "Rendered Heimdall logo SVGs.";
      license = licenses.asl20;
      platforms = platforms.linux;
    };
  };

  meta = with lib; {
    description = "Programmatic generator for the Heimdall project logo.";
    license = licenses.asl20;
    platforms = platforms.linux;
  };
})
