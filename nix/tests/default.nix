# Registry of Heimdall NixOS tests. flake.nix walks this attrset and
# exposes each entry as `checks.<system>.nixos-<name>`.
#
# `silicon-river` is intentionally NOT in the default registry: it requires
# a working spike + OpenOCD remote_bitbang JTAG handshake plus a River-
# compatible debug module model. Spike's stock JTAG DTM is a partial model
# and the bringup can stall on abstract commands the upstream ISS doesn't
# implement. Keep the file in-tree so the integration target stays visible,
# but exclude it from CI until the spike side is shimmed properly.
#
# Run it explicitly with:
#   nix build .#checks.<system>.nixos-silicon-river-experimental --no-link
{ self, pkgs }:
{
  module-basic = import ./module-basic.nix { inherit self pkgs; };
  daemon-api = import ./daemon-api.nix { inherit self pkgs; };
  silicon-river = import ./silicon-river.nix { inherit self pkgs; };
  silicon-aegis = import ./silicon-aegis.nix { inherit self pkgs; };
}
