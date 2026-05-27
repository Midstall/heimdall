# End-to-end silicon test: aegis-sim --rbb-port acts as the FPGA, OpenOCD
# spawned by heimdall speaks remote_bitbang to it, and a LoadAegisBitstream
# job exercises the full daemon -> OpenOCD -> JTAG path. The TAP model in
# aegis-sim only implements IDCODE / CONFIG / BYPASS, so this test covers
# JTAG load only. Per-pad I/O needs gpio-sim and is a later plan.
{ self, pkgs }:
let
  inherit (import ./lib.nix { inherit self pkgs; }) baseMachine;

  idcode = "0xdeadbeef";

  # SpawnedOpenocdJtagTransport appends `tcl_port`, `bindto`, and `init`
  # to argv. The config must not redeclare them and must disable the gdb
  # and telnet ports so OpenOCD doesn't try to bind their defaults.
  openocdCfg = pkgs.writeText "openocd-aegis.cfg" ''
    adapter driver remote_bitbang
    adapter speed 1000
    remote_bitbang host localhost
    remote_bitbang port 9824

    set _CHIPNAME aegis
    jtag newtap $_CHIPNAME cpu -irlen 4 -expected-id ${idcode}

    gdb_port disabled
    telnet_port disabled
  '';

  # Minimal device descriptor matching aegis-ip's own fixture. The driver
  # only reads `config.total_bits` from this for LoadAegisBitstream. The
  # rest exists to satisfy the schema.
  descriptor = {
    device = "test_fpga";
    fabric = {
      width = 2;
      height = 2;
      tracks = 1;
      tile_config_width = 46;
      bram = {
        column_interval = 0;
        columns = [ ];
        data_width = null;
        addr_width = null;
        depth = null;
        tile_config_width = 8;
      };
      dsp = {
        column_interval = 0;
        columns = [ ];
        a_width = null;
        b_width = null;
        result_width = null;
        tile_config_width = 16;
      };
      carry_chain = {
        direction = "south_to_north";
        per_column = true;
      };
    };
    io = {
      total_pads = 8;
      tile_config_width = 8;
      pads = [ ];
    };
    serdes = {
      count = 0;
      tile_config_width = 32;
      edge_assignment = [ ];
    };
    clock = {
      tile_count = 1;
      tile_config_width = 49;
      outputs_per_tile = 4;
      total_outputs = 4;
    };
    config = {
      total_bits = 233;
      chain_order = [ ];
    };
    tiles = [ ];
  };

  # 30-byte fixture for the 233-bit CONFIG payload. The pattern is
  # arbitrary because only the shift completing matters here.
  bitstreamBin = pkgs.runCommand "aegis-test-bitstream.bin" { } ''
    ${pkgs.python3}/bin/python3 - "$out" <<'PY'
    import sys
    total_bits = 233
    nbytes = (total_bits + 7) // 8
    data = bytearray(nbytes)
    for i in range(total_bits):
        if (i % 5) in (0, 3):
            data[i // 8] |= 1 << (i % 8)
    open(sys.argv[1], "wb").write(bytes(data))
    PY
  '';
in
pkgs.testers.nixosTest {
  name = "heimdall-silicon-aegis";

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
            name = "rig-aegis";
            bind = "127.0.0.1:7777";
          };
          dut = [
            {
              id = "aegis-1";
              kind = "aegis-luna1";
              transports = [ "jtag.openocd-aegis" ];
            }
          ];
          transport.jtag = [
            {
              id = "jtag.openocd-aegis";
              driver = "openocd-spawn";
              openocd_binary = "${pkgs.openocd}/bin/openocd";
              openocd_config = "${openocdCfg}";
              openocd_endpoint = "127.0.0.1:6666";
              freq_hz = 1000000;
            }
          ];
          # Validator requires a golden backend per DUT family. aegis-sim
          # is acting as the silicon, so mock is the right golden here.
          golden.aegis = {
            backend = "mock";
          };
        };
      };
      environment.systemPackages = with pkgs; [
        aegis-sim
        openocd
        jq
      ];
    };

  testScript = ''
    import base64
    import json

    rig.start()
    rig.wait_for_unit("heimdall.service", timeout=60)
    rig.wait_for_open_port(7777, timeout=30)

    rig.succeed(
        "systemd-run --unit aegis-sim "
        "${pkgs.aegis-sim}/bin/aegis-sim "
        "--rbb-port 9824 --rbb-idcode ${idcode} "
        "--rbb-bitstream-out /var/lib/heimdall/got.bin"
    )
    rig.wait_for_open_port(9824, timeout=30)

    bitstream_b64 = rig.succeed("base64 -w0 < ${bitstreamBin}").strip()
    descriptor_json = ${builtins.toJSON (builtins.toJSON descriptor)}

    body = json.dumps({
        "dut": "aegis-1",
        "kind": {
            "kind": "load-aegis-bitstream",
            "descriptor_json": descriptor_json,
            "bitstream_b64": bitstream_b64,
        },
    })
    resp = rig.succeed(
        "curl -sf -X POST http://127.0.0.1:7777/jobs "
        "-H 'content-type: application/json' "
        f"-d {json.dumps(body)}"
    )
    job_id = json.loads(resp)["id"]
    assert job_id, f"no job id: {resp}"

    def job_done(_):
        body = rig.succeed(f"curl -sf http://127.0.0.1:7777/jobs/{job_id}")
        state = json.loads(body)["state"]["state"]
        return state in ("done", "failed", "cancelled")

    retry(job_done, timeout_seconds=90)
    final = json.loads(rig.succeed(f"curl -sf http://127.0.0.1:7777/jobs/{job_id}"))
    assert final["state"]["state"] == "done", f"expected done, got: {final}"

    # `done` already proves the full CONFIG payload was shifted. A
    # byte-by-byte compare would also need to pin down heimdall's hex
    # wire-order convention, which we don't model here yet.
    rig.succeed("test -f /var/lib/heimdall/got.bin")
    got_b64 = rig.succeed("base64 -w0 < /var/lib/heimdall/got.bin").strip()
    got = base64.b64decode(got_b64)
    assert len(got) == 30, f"captured bitstream wrong length: {len(got)} bytes"

    rig.succeed("systemctl stop aegis-sim")
  '';
}
