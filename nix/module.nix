{ self }:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.mio;
  sessionArguments =
    [ "--backend" "udev" ]
    ++ lib.optionals cfg.xwayland.enable [
      "--xwayland-satellite"
      "--xwayland-display"
      cfg.xwayland.display
    ]
    ++ cfg.extraSessionArguments;
  escapedSessionArguments = lib.escapeShellArgs sessionArguments;
  sessionHelper = pkgs.writeShellScript "mio-session-helper" (''
    ${pkgs.dbus}/bin/dbus-update-activation-environment --systemd \
      WAYLAND_DISPLAY XDG_CURRENT_DESKTOP XDG_SESSION_DESKTOP XDG_SESSION_TYPE
  '' + lib.optionalString cfg.portal.enable ''
    ${pkgs.systemd}/bin/systemctl --user restart xdg-desktop-portal-wlr.service
    ${pkgs.systemd}/bin/systemctl --user restart xdg-desktop-portal.service
  '');
  sessionLauncher = pkgs.writeShellScript "mio-session" ''
    export MIO_SESSION_HELPER=${lib.escapeShellArg sessionHelper}
    exec ${lib.getExe cfg.package} ${escapedSessionArguments}
  '';
  sessionPackage = pkgs.runCommand "mio-wayland-session" {
    passthru.providedSessions = [ "mio" ];
  } ''
    install -Dm644 /dev/stdin $out/share/wayland-sessions/mio.desktop <<EOF
    [Desktop Entry]
    Name=Mio
    Comment=The Mio Wayland compositor
    Exec=${sessionLauncher}
    Type=Application
    DesktopNames=mio
    EOF
  '';
in
{
  options.programs.mio = {
    enable = lib.mkEnableOption "the Mio Wayland compositor and display-manager session";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      defaultText = lib.literalExpression "inputs.mio.packages.${pkgs.stdenv.hostPlatform.system}.default";
      description = "Mio package used by the desktop session.";
    };

    extraSessionArguments = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      example = [ "--config" "/home/alice/.config/mio/config.kdl" ];
      description = "Additional arguments passed to mio-compositor by the display-manager session.";
    };

    xwayland = {
      enable = lib.mkEnableOption "X11 compatibility through xwayland-satellite";

      display = lib.mkOption {
        type = lib.types.strMatching ":[0-9]+";
        default = ":100";
        description = "X display allocated to xwayland-satellite.";
      };
    };

    portal.enable = lib.mkEnableOption "xdg-desktop-portal-wlr integration" // {
      default = true;
    };

    recommendedPackages = lib.mkEnableOption "Foot, Waybar, and Wofi for the example configuration" // {
      default = true;
    };
  };

  config = lib.mkIf cfg.enable {
    environment.systemPackages =
      [ cfg.package ]
      ++ lib.optionals cfg.recommendedPackages [ pkgs.foot pkgs.waybar pkgs.wofi ]
      ++ lib.optionals cfg.xwayland.enable [ pkgs.xwayland-satellite ];

    services.displayManager.sessionPackages = [ sessionPackage ];
    hardware.graphics.enable = lib.mkDefault true;

    xdg.portal = lib.mkIf cfg.portal.enable {
      enable = true;
      config.mio = {
        default = lib.mkDefault [ "gtk" ];
        "org.freedesktop.impl.portal.ScreenCast" = lib.mkDefault [ "wlr" ];
        "org.freedesktop.impl.portal.Screenshot" = lib.mkDefault [ "wlr" ];
      };
      extraPortals = [
        pkgs.xdg-desktop-portal-gtk
        pkgs.xdg-desktop-portal-wlr
      ];
    };

    # NixOS' portal broker normally requires graphical-session.target. Mio keeps
    # application startup explicit, so permit only the broker to run without
    # activating the entire desktop autostart target.
    environment.etc = lib.mkIf cfg.portal.enable {
      "systemd/user/xdg-desktop-portal.service.d/mio.conf".text = ''
        [Unit]
        Requisite=
      '';
    };
  };
}
