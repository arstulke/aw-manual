# Home Manager module: installs aw-manual, optionally manages config.yaml and
# favorites files, and starts the daemon with the graphical session.
self:
{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.aw-manual;
  yaml = pkgs.formats.yaml { };
in
{
  options.services.aw-manual = {
    enable = lib.mkEnableOption "aw-manual, manual activity tracking for ActivityWatch";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "aw-manual.packages.\${system}.default";
      description = "The aw-manual package.";
    };

    settings = lib.mkOption {
      type = yaml.type;
      default = { };
      example = lib.literalExpression ''
        {
          server.url = "http://localhost:5600";
          hotkeys.popover = "Mouse4";
          tray.display = "elapsed";
        }
      '';
      description = ''
        Content of `config.yaml`. When set, the file is managed by Home Manager
        and changes made in the settings window are not kept.
        Leave empty to manage the file with the settings window instead.
      '';
    };

    favorites = lib.mkOption {
      type = lib.types.attrsOf yaml.type;
      default = { };
      example = lib.literalExpression ''
        {
          "50-nix" = {
            favorites = [
              { id = "acme"; title = "ACME"; color = "#E4572E"; children = [
                { id = "dev"; title = "Development"; }
                { id = "review"; title = "Code Review"; }
              ]; }
              { id = "break"; title = "Break"; kind = "status"; }
            ];
          };
        }
      '';
      description = ''
        Favorites files written to `favorites.d/<name>.yaml`. They are marked
        `generated: true` (read-only in the settings window). Nix attribute
        sets are sorted by name, so use the explicit notation (lists) to keep
        your order.
      '';
    };

    autostart = lib.mkOption {
      type = lib.types.bool;
      default = true;
      description = "Start the aw-manual daemon with the graphical session (systemd user service).";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ];

    xdg.configFile =
      lib.optionalAttrs (cfg.settings != { }) {
        "aw-manual/config.yaml".source = yaml.generate "aw-manual-config.yaml" cfg.settings;
      }
      // lib.mapAttrs' (
        name: value:
        lib.nameValuePair "aw-manual/favorites.d/${name}.yaml" {
          source = yaml.generate "aw-manual-${name}.yaml" ({ generated = true; } // value);
        }
      ) cfg.favorites;

    systemd.user.services.aw-manual = lib.mkIf cfg.autostart {
      Unit = {
        Description = "aw-manual daemon (manual activity tracking for ActivityWatch)";
        PartOf = [ "graphical-session.target" ];
        After = [ "graphical-session.target" ];
      };
      Service = {
        ExecStart = "${cfg.package}/bin/aw-manual-daemon --background";
        Restart = "on-failure";
      };
      Install.WantedBy = [ "graphical-session.target" ];
    };
  };
}
