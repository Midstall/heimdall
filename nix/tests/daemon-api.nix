# End-to-end exercise of the HTTP API: submit a mock-hello job against a
# configured mock DUT, watch it transition to done/pass, then verify the
# /metrics counters and the Japanese /i18n.json catalog.
{ self, pkgs }:
let
  inherit (import ./lib.nix { inherit self pkgs; }) baseMachine;
in
pkgs.testers.nixosTest {
  name = "heimdall-daemon-api";

  nodes.rig =
    { ... }:
    {
      imports = [ baseMachine ];
      services.heimdall = {
        enable = true;
        bind = "127.0.0.1:7777";
        settings = {
          host = {
            name = "rig-api";
            bind = "127.0.0.1:7777";
          };
          dut = [
            {
              id = "mock-1";
              kind = "river-rc1-nano";
              transports = [ "jtag.mock" ];
            }
          ];
          transport.jtag = [
            {
              id = "jtag.mock";
              driver = "mock";
              freq_hz = 1000000;
            }
          ];
          # Validator requires a golden backend per DUT family; mock-1
          # is in the River family.
          golden.river = {
            backend = "mock";
          };
        };
      };
      environment.systemPackages = [ pkgs.jq ];
    };

  testScript = ''
    import json

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

    # The DUT registry should report mock-1 with the mock JTAG driver.
    rig.wait_until_succeeds("curl -sf http://127.0.0.1:7777/duts", timeout=15)
    duts = json.loads(rig.succeed("curl -sf http://127.0.0.1:7777/duts"))
    assert len(duts["duts"]) == 1, f"expected 1 DUT, got: {duts}"
    assert duts["duts"][0]["id"] == "mock-1", f"unexpected: {duts}"

    # Submit a MockHello job.
    body = json.dumps({"dut": "mock-1", "kind": {"kind": "mock-hello"}})
    resp = rig.succeed(
        "curl -sf -X POST http://127.0.0.1:7777/jobs "
        "-H 'content-type: application/json' "
        f"-d {json.dumps(body)}"
    )
    job = json.loads(resp)
    job_id = job["id"]
    assert job_id, f"no job id: {resp}"

    # Wait for terminal state. Use Python-driven polling so we get a clear
    # timeout message rather than hanging.
    def job_done(_):
        body = rig.succeed(f"curl -sf http://127.0.0.1:7777/jobs/{job_id}")
        state = json.loads(body)["state"]["state"]
        return state in ("done", "failed", "cancelled")

    try:
        retry(job_done, timeout_seconds=30)
    except Exception:
        dump_logs()
        raise

    final = json.loads(rig.succeed(f"curl -sf http://127.0.0.1:7777/jobs/{job_id}"))
    assert final["state"]["state"] == "done", f"expected done, got: {final}"
    assert final["state"]["detail"]["kind"] == "pass", f"expected pass, got: {final}"

    # /metrics reports the verdict counter.
    metrics = rig.succeed("curl -sf http://127.0.0.1:7777/metrics")
    assert 'heimdall_verdicts{result="pass"} 1' in metrics, \
        f"pass verdict not counted: {metrics}"

    # /i18n.json returns Japanese when requested.
    ja = rig.succeed("curl -sf 'http://127.0.0.1:7777/i18n.json?lang=ja'")
    assert "ジョブ" in ja, "ja catalog missing"
  '';
}
