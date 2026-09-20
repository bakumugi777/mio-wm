{
  description = "Mio — a Smithay Wayland compositor with one continuous 2D world";

  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
    in
    {
      packages = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.callPackage ./nix/package.nix { };
          mio = self.packages.${system}.default;
        });

      devShells = forAllSystems (system:
        let pkgs = nixpkgs.legacyPackages.${system};
        in {
          default = pkgs.callPackage ./shell.nix { };
        });

      nixosModules = {
        default = import ./nix/module.nix { inherit self; };
        mio = self.nixosModules.default;
      };

      checks = forAllSystems (system:
        let
          pkgs = nixpkgs.legacyPackages.${system};
          testSystem = nixpkgs.lib.nixosSystem {
            inherit system;
            modules = [
              self.nixosModules.default
              {
                programs.mio.enable = true;
                programs.mio.xwayland.enable = true;
                system.stateVersion = "26.05";
                fileSystems."/" = {
                  device = "none";
                  fsType = "tmpfs";
                };
                boot.loader.grub.enable = false;
              }
            ];
          };
          sessionPackage = builtins.head testSystem.config.services.displayManager.sessionPackages;
        in {
          package = self.packages.${system}.default;
          installer = pkgs.runCommand "mio-installer-check" {
            nativeBuildInputs = [ pkgs.shellcheck ];
          } ''
            shellcheck ${./install.sh}
            touch $out
          '';
          nixos-module = pkgs.runCommand "mio-nixos-module-check" { } ''
            test "${builtins.head sessionPackage.providedSessions}" = mio
            test "${testSystem.config.xdg.portal.config.mio."org.freedesktop.impl.portal.ScreenCast"}" = wlr
            test "${testSystem.config.xdg.portal.config.mio."org.freedesktop.impl.portal.Secret"}" = gnome-keyring
            test "${toString testSystem.config.services.gnome.gnome-keyring.enable}" = 1
            test "${testSystem.config.systemd.user.services.mio-xdg-desktop-portal.serviceConfig.BusName}" = org.freedesktop.portal.Desktop
            touch $out
          '';
        });
    };
}
