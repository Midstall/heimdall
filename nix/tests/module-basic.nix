# Smallest possible test: enable the module with defaults, verify the
# daemon comes up and /health responds.
{ self, pkgs }:
let
  inherit (import ./lib.nix { inherit self pkgs; }) baseMachine;
in
pkgs.testers.nixosTest {
  name = "heimdall-module-basic";

  nodes.rig =
    { ... }:
    {
      imports = [ baseMachine ];
      services.heimdall = {
        enable = true;
        bind = "127.0.0.1:7777";
      };
    };

  testScript = ''
    def dump_logs():
        print("===== heimdall.service journal =====")
        print(rig.execute("journalctl -u heimdall -n 80 --no-pager --output=cat")[1])
        print("===== systemctl status =====")
        print(rig.execute("systemctl status heimdall --no-pager")[1])

    rig.start()
    try:
        rig.wait_for_unit("heimdall.service", timeout=60)
        rig.wait_for_open_port(7777, timeout=30)
    except Exception:
        dump_logs()
        raise

    # /health is the cheapest endpoint that proves the router is up.
    health = rig.succeed("curl -sf http://127.0.0.1:7777/health")
    assert '"status":"ok"' in health, f"unexpected /health body: {health}"

    # Service user owns its data dir.
    rig.succeed("getent passwd heimdall")
    rig.succeed("test -d /var/lib/heimdall")

    # udev rules deployed.
    rig.succeed("grep -q 'idVendor.*0403' /etc/udev/rules.d/99-local.rules")

    # SSR'd index includes the favicon link and the page title.
    index = rig.succeed("curl -sf http://127.0.0.1:7777/")
    assert 'rel="icon"' in index, "favicon link missing from SSR HTML"
    assert ">Heimdall</title>" in index, "title not rendered"

    # Favicon is actually served.
    fav = rig.succeed("curl -sf http://127.0.0.1:7777/assets/favicon.svg")
    assert fav.startswith("<?xml") and "<svg" in fav, "favicon SVG malformed"
  '';
}
