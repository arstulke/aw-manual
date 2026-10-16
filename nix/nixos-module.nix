# NixOS module: installs aw-manual and starts the daemon with the graphical session.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.programs.aw-manual;
in
{
  options.programs.aw-manual = {
    enable = lib.mkEnableOption "aw-manual, manual activity tracking for ActivityWatch";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "aw-manual.packages.\${system}.default";
      description = "The aw-manual package.";
    };

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Start the aw-manual daemon with the graphical session (systemd user service).";
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages = [ cfg.package ];

    systemd.user.services.aw-manual = lib.mkIf cfg.autostart {
      description = "aw-manual daemon (manual activity tracking for ActivityWatch)";
      partOf = [ "graphical-session.target" ];
      after = [ "graphical-session.target" ];
      wantedBy = [ "graphical-session.target" ];
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/aw-manual-daemon --background";
        Restart = "on-failure";
      };
    };
  };
}
