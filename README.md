# Heimdall

Post-silicon hardware verification suite. Drives real chips on the bench, compares them to a golden reference model, and fuzzes them with coverage feedback.

## Features

- **Hardware-in-the-loop test runner.** `TestDriver` trait plus built-in drivers for the [River](https://github.com/Midstall/river) RISC-V CPU, the [Aegis](https://github.com/Midstall/aegis) FPGA fabric, and a mock for offline development.
- **Golden reference models.** spike for RISC-V, the native Rust Aegis simulator, and ngspice for analog blocks. Per-iteration sim-vs-silicon divergence detection.
- **Coverage-guided fuzzer.** Cranelift-backed structured codegen for RV64, raw-encoder fallback for corner cases, power-scheduled by novel coverage on both sim and silicon.
- **JTAG transports.** Spawned or attached OpenOCD, Linux GPIO bit-bang (for Pi-as-host deployment), and a mock layer for unit tests.
- **Acceptance campaigns.** Production-pipeline ready: `bringup`, `fuzz`, and custom templates per DUT family, emitted as machine-readable JSON reports.
- **Daemon with web UI.** axum HTTP + WebSocket server, sqlite job store, local-fs blob store, embedded vanilla-JS web UI with Tokyo Night styling. SPICE netlist rendering with coverage overlay.
- **Terminal UI.** ratatui front-end that talks to the daemon over HTTP + WS.
- **Library-first.** Every layer is a real Rust crate. Embed Heimdall in your own bringup tooling without going through the CLI.

## Install

Nix is the recommended path:

```sh
nix profile install github:Midstall/heimdall
```

For development:

```sh
nix develop
cargo build --release --features fuzzer,daemon,tui,aegis,river
```

### NixOS module (test bench rigs)

The flake exposes a `nixosModules.default` that runs the daemon as a systemd
service with the right hardware-access plumbing (udev rules for FTDI / J-Link /
ST-Link / Picoprobe / USB-Blaster, optional GPIO group, dedicated service user,
sandboxed unit). Add the input and import the module:

```nix
{
  inputs.heimdall.url = "github:Midstall/heimdall";

  outputs = { self, nixpkgs, heimdall }: {
    nixosConfigurations.lab-rig-1 = nixpkgs.lib.nixosSystem {
      system = "aarch64-linux";
      modules = [
        heimdall.nixosModules.default
        ({ ... }: {
          services.heimdall = {
            enable = true;
            bind = "0.0.0.0:7777";
            openFirewall = true;
            gpio.enable = true;              # bit-bang JTAG on Pi
            settings = {
              host = { name = "lab-rig-1"; bind = "0.0.0.0:7777"; };
              dut = [{
                id = "river-1";
                kind = "river-rc1-nano";
                transports = [ "jtag.ftdi-1" ];
              }];
              transport.jtag = [{
                id = "jtag.ftdi-1";
                driver = "ftdi";
                ftdi_vid = 1027;             # 0x0403
                ftdi_pid = 24592;            # 0x6010 (FT2232H)
                freq_hz = 1000000;
              }];
            };
          };
        })
      ];
    };
  };
}
```

The full option set (custom udev rules, alternate users/groups, hardening
overrides) is documented in `nix/modules/nixos.nix`.

Heimdall is trusted-environment only: the daemon ships without auth, so keep
non-loopback binds on isolated lab networks.

### home-manager module (per-user CLI)

For per-user setup (CLI, default daemon URL, rendered `heimdall.toml`):

```nix
{ heimdall, ... }: {
  imports = [ heimdall.homeManagerModules.default ];

  programs.heimdall = {
    enable = true;
    daemonUrl = "http://lab-rig-1:7777";
    settings = {
      host = { name = "in-field-rig"; bind = "127.0.0.1:7777"; };
      # ...same dut / transport shape as above.
    };
  };
}
```

This installs the `heimdall` binary into the user environment, renders the
settings to `~/.config/heimdall/heimdall.toml`, and exports
`HEIMDALL_DAEMON_URL`.

## Quickstart

Heimdall ships a mock pipeline so you can exercise the full stack without hardware:

```sh
# Run the full compile -> load -> run -> observe -> diff pipeline against mocks.
cargo run -p heimdall-test --example mock_bringup

# Run the coverage-guided fuzzer for 100 iterations against the same mocks.
cargo run -p heimdall-fuzzer --example mock_fuzz -- 100

# Verify your environment is set up for real hardware.
heimdall doctor

# Start the daemon and drive it from the web UI / TUI.
heimdall daemon serve --bind 127.0.0.1:8080 &
heimdall tui --daemon http://127.0.0.1:8080
```

Open `http://127.0.0.1:8080/` for the web UI.

## Configuration

Heimdall reads `heimdall.toml` from CWD (or `--config <path>`). It describes:

- The host (bind address, name).
- One or more DUTs, each with a kind, transports, golden model, optional SPICE netlist, and per-DUT bringup vector.
- Transport definitions: JTAG (OpenOCD, bit-bang, mock), serial, and GPIO.
- Tool paths: clang, spike, openocd.

See `crates/heimdall-config/testdata/example.toml` for a working reference and `crates/heimdall-config/src/schema.rs` for the full schema.

## Library usage

```rust
use heimdall_core::{DutId, DutKind, State, ValueRepr};
use heimdall_driver::{Dut, MockDriver};
use heimdall_golden::MockGoldenModel;
use heimdall_test::Runner;

let runner = Runner::builder().build();
let mut driver = MockDriver::new(DutKind::RiverRc1Nano)
    .with_state(State::new().with("a0", ValueRepr::U64(0x42)));
let mut golden = MockGoldenModel::new(DutKind::RiverRc1Nano);
let mut dut = Dut::new(DutId::new("d1"), DutKind::RiverRc1Nano);

let result = runner.run_one(&my_test, &mut dut, &mut driver, &mut golden).await?;
println!("{:?}", result.verdict);
```

See `crates/heimdall-test/examples/mock_bringup.rs` for a complete runnable example.

Every crate in `crates/` is publishable on its own. Build only what you need.

## Crates

| Crate                | Role                                              |
|----------------------|---------------------------------------------------|
| `heimdall`           | Umbrella re-export.                               |
| `heimdall-core`      | IDs, verdicts, observations, stimuli, artifacts.  |
| `heimdall-config`    | TOML schema + validation.                         |
| `heimdall-transport` | JTAG, serial, GPIO, OpenOCD, mock.                |
| `heimdall-tools`     | clang / spike / openocd wrappers.                 |
| `heimdall-golden`    | spike + Aegis-sim + SPICE/ngspice golden models.  |
| `heimdall-driver`    | `TestDriver` trait, River / Aegis / Mock drivers. |
| `heimdall-test`      | Runner + campaign framework.                      |
| `heimdall-fuzzer`    | Coverage-guided fuzzer.                           |
| `heimdall-daemon`    | HTTP/WS daemon + web UI.                          |
| `heimdall-tui`       | ratatui front-end.                                |
| `heimdall-eda`       | `heimdall` CLI binary.                            |

## Commercial support

Midstall offers paid integration work for Heimdall: new DUT drivers, custom transports, on-prem deployment, and bringup-as-a-service. Contact `inquire@midstall.com`.
