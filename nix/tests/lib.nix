# Shared helpers for Heimdall NixOS tests. Each test imports this to keep
# the per-test boilerplate small.
{ self, pkgs }:
let
  inherit (pkgs) lib;
in
{
  # Common module config a test machine gets: import the Heimdall NixOS
  # module and the overlay providing the daemon package.
  baseMachine =
    { ... }:
    {
      imports = [ self.nixosModules.heimdall ];
      nixpkgs.overlays = [ self.overlays.default ];
      # Default mem; tests with heavier needs (e.g. silicon-river) raise
      # this with `lib.mkForce`.
      virtualisation.memorySize = lib.mkDefault 1024;
    };

  # Pythonic shorthand for invoking the test script.
  inherit lib;
}
