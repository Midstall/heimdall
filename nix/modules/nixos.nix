{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.heimdall;

  # The heimdall.toml the daemon reads. If `cfg.settings` is set, render it
  # to a file; otherwise let `cfg.configFile` point at an existing path; if
  # neither is set, the daemon starts with an empty DUT registry.
  settingsFile =
    if cfg.settings != null then
      (pkgs.formats.toml { }).generate "heimdall.toml" cfg.settings
    else
      cfg.configFile;

  port = lib.toInt (lib.elemAt (lib.splitString ":" cfg.bind) 1);

  # Udev rules that grant access to the currently-logged-in seat user via
  # logind's uaccess tag. This is the same mechanism Arduino IDE, OpenOCD,
  # J-Link, and ST-Link use. No `plugdev` group required.
  udevRulesText = lib.concatStringsSep "\n" (
    [
      # FTDI (FT232R, FT232H, FT2232C/D/L, FT2232H, FT4232H, FT2232HP, etc).
      ''SUBSYSTEM=="usb", ATTR{idVendor}=="0403", MODE="0660", TAG+="uaccess"''
      # USB-Blaster (Altera/Intel JTAG; rare but cheap).
      ''SUBSYSTEM=="usb", ATTR{idVendor}=="09fb", MODE="0660", TAG+="uaccess"''
      # CMSIS-DAP / J-Link / ST-Link, common debug probes.
      ''SUBSYSTEM=="usb", ATTR{idVendor}=="1366", MODE="0660", TAG+="uaccess"'' # SEGGER J-Link
      ''SUBSYSTEM=="usb", ATTR{idVendor}=="0483", MODE="0660", TAG+="uaccess"'' # ST-Link
      ''SUBSYSTEM=="usb", ATTR{idVendor}=="03eb", MODE="0660", TAG+="uaccess"'' # Atmel/Microchip
      ''SUBSYSTEM=="usb", ATTR{idVendor}=="2e8a", MODE="0660", TAG+="uaccess"'' # Raspberry Pi (Pico/Picoprobe)
    ]
    ++ lib.optionals cfg.gpio.enable [
      # Linux GPIO character device, used by the bit-bang JTAG transport.
      ''SUBSYSTEM=="gpio", KERNEL=="gpiochip*", GROUP="${cfg.gpio.group}", MODE="0660"''
    ]
    ++ cfg.extraUdevRules
  );

  daemonCommand = lib.escapeShellArgs (
    [
      (lib.getExe cfg.package)
      "daemon"
      "serve"
      "--bind"
      cfg.bind
      "--store-path"
      "${cfg.dataDir}/heimdall.db"
      "--blob-path"
      "${cfg.dataDir}/objects"
    ]
    ++ lib.optionals (settingsFile != null) [
      "--config"
      "${settingsFile}"
    ]
  );
in
{
  options.services.heimdall = {
    enable = lib.mkEnableOption "the Heimdall hardware verification daemon";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "heimdall.packages.\${system}.default";
      description = "The heimdall CLI package providing the `heimdall daemon serve` entry point.";
    };

    bind = lib.mkOption {
      type = lib.types.str;
      default = "127.0.0.1:7777";
      example = "0.0.0.0:7777";
      description = ''
        Address and port the daemon binds to. The daemon has no auth by
        design (`trusted-environment only`), so non-loopback binds should
        only be used on isolated lab networks.
      '';
    };

    openFirewall = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Open the daemon's TCP port in the system firewall.";
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/heimdall";
      description = "Directory for the sqlite JobStore database and the local-fs BlobStore.";
    };

    user = lib.mkOption {
      type = lib.types.str;
      default = "heimdall";
      description = "User the daemon runs as.";
    };

    group = lib.mkOption {
      type = lib.types.str;
      default = "heimdall";
      description = "Primary group for the daemon user. Also receives uaccess on hardware devices.";
    };

    extraGroups = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ "dialout" ];
      description = ''
        Additional groups the daemon user is a member of. Defaults to
        `dialout` so the daemon can read serial-attached UART probes; add
        `gpio` here if you set `services.heimdall.gpio.enable`.
      '';
    };

    configFile = lib.mkOption {
      type = lib.types.nullOr lib.types.path;
      default = null;
      description = ''
        Path to a pre-existing heimdall.toml. Mutually exclusive with
        `settings`. If both are null, the daemon starts with an empty
        DUT registry.
      '';
    };

    settings = lib.mkOption {
      type = lib.types.nullOr (pkgs.formats.toml { }).type;
      default = null;
      example = lib.literalExpression ''
        {
          host = { name = "rig-1"; bind = "127.0.0.1:7777"; };
          dut = [{
            id = "mock-1";
            kind = "river-rc1-nano";
            transports = [ "jtag.mock" ];
          }];
          transport.jtag = [{ id = "jtag.mock"; driver = "mock"; freq_hz = 1000000; }];
        }
      '';
      description = ''
        heimdall.toml content as a Nix attrset; rendered to a TOML file
        and passed to the daemon via `--config`. See
        `crates/heimdall-config/src/schema.rs` for the full schema.
      '';
    };

    gpio = {
      enable = lib.mkOption {
        type = lib.types.bool;
        default = false;
        description = ''
          Set up GPIO character-device access (for bit-bang JTAG on
          Raspberry Pi or similar SBCs). Creates the `gpio` group and a
          udev rule chowning `/dev/gpiochip*` to it.
        '';
      };
      group = lib.mkOption {
        type = lib.types.str;
        default = "gpio";
        description = "Group that owns /dev/gpiochip* when gpio.enable is true.";
      };
    };

    extraUdevRules = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [
        ''SUBSYSTEM=="usb", ATTR{idVendor}=="2a03", MODE="0660", TAG+="uaccess"''
      ];
      description = ''
        Additional udev rules appended to the Heimdall hardware-access set.
        Useful for non-standard probes or scope/PSU instruments.
      '';
    };

    environment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        RUST_LOG = "heimdall=debug,info";
      };
      description = "Extra environment variables for the systemd unit.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.settings != null && cfg.configFile != null);
        message = "services.heimdall: set either `settings` or `configFile`, not both.";
      }
      {
        assertion =
          (lib.length (lib.splitString ":" cfg.bind) == 2)
          && (lib.match "[0-9]+" (lib.elemAt (lib.splitString ":" cfg.bind) 1) != null);
        message = "services.heimdall.bind must be of the form `host:port`.";
      }
    ];

    environment.systemPackages = [ cfg.package ];

    users.users.${cfg.user} = {
      isSystemUser = true;
      group = cfg.group;
      extraGroups = cfg.extraGroups ++ lib.optional cfg.gpio.enable cfg.gpio.group;
      home = cfg.dataDir;
      createHome = false;
      description = "Heimdall daemon service user";
    };
    users.groups.${cfg.group} = { };
    users.groups.${cfg.gpio.group} = lib.mkIf cfg.gpio.enable { };

    services.udev.extraRules = udevRulesText;

    systemd.tmpfiles.rules = [
      "d ${cfg.dataDir} 0750 ${cfg.user} ${cfg.group} - -"
      "d ${cfg.dataDir}/objects 0750 ${cfg.user} ${cfg.group} - -"
    ];

    systemd.services.heimdall = {
      description = "Heimdall hardware verification daemon";
      after = [ "network.target" ];
      wantedBy = [ "multi-user.target" ];

      environment = {
        RUST_LOG = "info";
      }
      // cfg.environment;

      serviceConfig = {
        Type = "simple";
        ExecStart = daemonCommand;
        User = cfg.user;
        Group = cfg.group;
        WorkingDirectory = cfg.dataDir;
        StateDirectory = "heimdall";
        Restart = "on-failure";
        RestartSec = 5;

        # Hardening. Daemon is trusted-environment only but we still scope
        # what it can do to the parts it needs.
        ProtectSystem = "strict";
        ProtectHome = true;
        ReadWritePaths = [ cfg.dataDir ];
        PrivateTmp = true;
        NoNewPrivileges = true;
        ProtectKernelTunables = true;
        ProtectKernelModules = true;
        ProtectControlGroups = true;
        RestrictRealtime = true;
        RestrictSUIDSGID = true;
        # Need raw USB + tty + gpiochip access for real transports.
        DeviceAllow = [
          "char-usb_device rw"
          "char-tty rw"
          "char-gpio rw"
        ];
      };
    };

    networking.firewall.allowedTCPPorts = lib.mkIf cfg.openFirewall [ port ];
  };
}
