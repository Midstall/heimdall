{ self }:
{
  config,
  lib,
  pkgs,
  ...
}:

let
  cfg = config.services.heimdall;

  settingsFile =
    if cfg.settings != null then
      (pkgs.formats.toml { }).generate "heimdall.toml" cfg.settings
    else
      cfg.configFile;
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
      # Backwards-compat: existing string-form configs auto-wrap to a
      # one-element list, so a `bind = "127.0.0.1:7777"` already in the
      # wild keeps working unchanged.
      type = lib.types.coercedTo lib.types.str (s: [ s ]) (lib.types.listOf lib.types.str);
      default = [ "[::]:7777" ];
      example = [
        "127.0.0.1:7777"
        "10.0.1.5:8080"
      ];
      description = ''
        Addresses and ports the daemon binds to. Default `[::]:7777`
        accepts both IPv4 and IPv6 traffic through the same dual-stack
        socket on most platforms. The daemon has no auth by design, so
        non-loopback binds should only be used on isolated lab
        networks. Specify multiple entries to listen on more than one
        explicit interface.
      '';
    };

    dataDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/lib/heimdall";
      description = "Directory for the sqlite JobStore database and the local-fs BlobStore.";
    };

    logDir = lib.mkOption {
      type = lib.types.path;
      default = "/var/log";
      description = "Directory launchd writes the daemon's stdout/stderr into.";
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
      description = ''
        heimdall.toml content as a Nix attrset. Rendered to a TOML file
        and passed to the daemon via `--config`. See
        `crates/heimdall-config/src/schema.rs` for the full schema.
      '';
    };

    environment = lib.mkOption {
      type = lib.types.attrsOf lib.types.str;
      default = { };
      example = {
        RUST_LOG = "heimdall=debug,info";
      };
      description = "Extra environment variables for the launchd job.";
    };
  };

  config = lib.mkIf cfg.enable {
    assertions = [
      {
        assertion = !(cfg.settings != null && cfg.configFile != null);
        message = "services.heimdall: set either `settings` or `configFile`, not both.";
      }
      {
        assertion = cfg.bind != [ ];
        message = "services.heimdall.bind must contain at least one address.";
      }
      {
        assertion = lib.all (b: lib.match ".*:[0-9]+" b != null) cfg.bind;
        message = "services.heimdall.bind entries must each be of the form `host:port` or `[v6]:port`.";
      }
    ];

    environment.systemPackages = [ cfg.package ];

    launchd.daemons.heimdall = {
      script = ''
        mkdir -p ${cfg.dataDir} ${cfg.dataDir}/objects
        exec ${lib.getExe cfg.package} daemon serve \
          ${lib.concatMapStringsSep " \\\n          " (b: "--bind ${lib.escapeShellArg b}") cfg.bind} \
          --store-path ${cfg.dataDir}/heimdall.db \
          --blob-path ${cfg.dataDir}/objects \
          ${lib.optionalString (settingsFile != null) "--config ${settingsFile}"}
      '';

      environment = {
        RUST_LOG = "info";
      }
      // cfg.environment;

      serviceConfig = {
        Label = "com.midstall.heimdall";
        RunAtLoad = true;
        KeepAlive = true;
        StandardOutPath = "${cfg.logDir}/heimdall.out.log";
        StandardErrorPath = "${cfg.logDir}/heimdall.err.log";
        WorkingDirectory = toString cfg.dataDir;
      };
    };
  };
}
