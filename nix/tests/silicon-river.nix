# End-to-end silicon test: spike acts as the CPU via `--rbb-port`, OpenOCD
# spawned by heimdall speaks remote_bitbang to it, and a `BootRiverElf`
# job exercises the full daemon -> OpenOCD -> JTAG -> hart path. Spike's
# JTAG DTM is only a partial Debug Module model, so some River-specific
# abstract-command paths aren't covered.
{ self, pkgs }:
let
  inherit (import ./lib.nix { inherit self pkgs; }) baseMachine;

  # SpawnedOpenocdJtagTransport appends `tcl_port`, `bindto`, and `init`
  # to argv, so this file must not declare them. `gdb_port`/`telnet_port`
  # are disabled because OpenOCD would otherwise bind their defaults.
  openocdCfg = pkgs.writeText "openocd-spike.cfg" ''
    adapter driver remote_bitbang
    adapter speed 1000
    remote_bitbang host localhost
    remote_bitbang port 9824

    set _CHIPNAME riscv
    jtag newtap $_CHIPNAME cpu -irlen 5

    set _TARGETNAME $_CHIPNAME.cpu
    target create $_TARGETNAME riscv -chain-position $_TARGETNAME

    gdb_port disabled
    telnet_port disabled
  '';

  helloElf = ../../testdata/river-bringup/hello.elf;
in
pkgs.testers.nixosTest {
  name = "heimdall-silicon-river";

  nodes.rig =
    { ... }:
    {
      imports = [ baseMachine ];
      virtualisation.memorySize = pkgs.lib.mkForce 2048;
      services.heimdall = {
        enable = true;
        bind = "127.0.0.1:7777";
        settings = {
          host = {
            name = "rig-river";
            bind = "127.0.0.1:7777";
          };
          dut = [
            {
              id = "river-1";
              kind = "river-rc1-nano";
              transports = [ "jtag.openocd-spike" ];
            }
          ];
          transport.jtag = [
            {
              id = "jtag.openocd-spike";
              driver = "openocd-spawn";
              openocd_binary = "${pkgs.openocd}/bin/openocd";
              openocd_config = "${openocdCfg}";
              openocd_endpoint = "127.0.0.1:6666";
              freq_hz = 1000000;
            }
          ];
          # Validator requires a golden backend for River-family DUTs.
          # Mock is fine here because spike is acting as the silicon.
          golden.river = {
            backend = "mock";
          };
        };
      };
      environment.systemPackages = with pkgs; [
        spike
        openocd
        dtc
        jq
      ];
    };

  testScript = ''
    import json

    rig.start()
    rig.wait_for_unit("heimdall.service", timeout=60)
    rig.wait_for_open_port(7777, timeout=30)

    rig.succeed("install -Dm0644 ${helloElf} /var/lib/heimdall/hello.elf")

    # `-m` overrides spike's default 2 GiB-at-0x80000000 layout to make
    # room at 0x10000 where hello.elf links. dtc must be on PATH because
    # spike shells out to it at startup.
    rig.succeed(
        "systemd-run --unit spike "
        "--setenv=PATH=${pkgs.spike}/bin:${pkgs.dtc}/bin "
        "${pkgs.spike}/bin/spike --rbb-port=9824 "
        "-m0x10000:0x100000,0x80000000:0x10000000 -H "
        "/var/lib/heimdall/hello.elf"
    )
    rig.wait_for_open_port(9824, timeout=30)

    elf_b64 = rig.succeed("base64 -w0 < /var/lib/heimdall/hello.elf").strip()
    body = json.dumps({
        "dut": "river-1",
        "kind": {"kind": "boot-river-elf", "elf_b64": elf_b64, "cycles": 2000},
    })
    resp = rig.succeed(
        "curl -sf -X POST http://127.0.0.1:7777/jobs "
        "-H 'content-type: application/json' "
        f"-d {json.dumps(body)}"
    )
    job_id = json.loads(resp)["id"]
    assert job_id, f"no job id: {resp}"

    # OpenOCD startup plus JTAG handshake is slower than mock paths.
    def job_done(_):
        body = rig.succeed(f"curl -sf http://127.0.0.1:7777/jobs/{job_id}")
        state = json.loads(body)["state"]["state"]
        return state in ("done", "failed", "cancelled")

    retry(job_done, timeout_seconds=90)
    final = json.loads(rig.succeed(f"curl -sf http://127.0.0.1:7777/jobs/{job_id}"))
    assert final["state"]["state"] == "done", f"expected done, got: {final}"

    rig.succeed("systemctl stop spike")
  '';
}
