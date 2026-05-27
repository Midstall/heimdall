{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.programs.heimdall;
  tomlFormat = pkgs.formats.toml { };
in
{
  options.programs.heimdall = {
    enable = lib.mkEnableOption "the Heimdall hardware verification CLI";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "heimdall.packages.\${system}.default";
      description = "The heimdall CLI package.";
    };

    settings = lib.mkOption {
      type = lib.types.nullOr tomlFormat.type;
      default = null;
      example = lib.literalExpression ''
        {
          host = { name = "in-field-rig"; bind = "127.0.0.1:7777"; };
          dut = [{
            id = "ft2232h-1";
            kind = "river-rc1-nano";
            transports = [ "jtag.ftdi-main" ];
          }];
          transport.jtag = [{
            id = "jtag.ftdi-main";
            driver = "ftdi";
            ftdi_vid = 1027;     # 0x0403
            ftdi_pid = 24592;    # 0x6010 (FT2232H)
            ftdi_interface = 0;
            freq_hz = 1000000;
          }];
        }
      '';
      description = ''
        heimdall.toml content. Rendered to
        `$XDG_CONFIG_HOME/heimdall/heimdall.toml` (defaults to
        `~/.config/heimdall/heimdall.toml`), which is the path the CLI
        looks at by default. Set the `HEIMDALL_CONFIG` env var or pass
        `--config` to override.
      '';
    };

    daemonUrl = lib.mkOption {
      type = lib.types.nullOr lib.types.str;
      default = null;
      example = "http://lab-rig-1:7777";
      description = ''
        Default daemon URL passed to subcommands that need to talk to a
        running daemon (`heimdall tui`, `heimdall campaign get`, etc).
        Exported as the `HEIMDALL_DAEMON_URL` environment variable.
      '';
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile."heimdall/heimdall.toml" = lib.mkIf (cfg.settings != null) {
      source = tomlFormat.generate "heimdall.toml" cfg.settings;
    };

    home.sessionVariables = lib.mkIf (cfg.daemonUrl != null) {
      HEIMDALL_DAEMON_URL = cfg.daemonUrl;
    };
  };
}
