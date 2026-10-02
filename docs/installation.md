# Installation

[日本語版](installation-jp.md)

Mio provides a Nix flake package, a NixOS module, and a portable installer. Enabling the module registers a Wayland session that can be selected in SDDM and other display managers.

> [!WARNING]
> Mio has currently been tested on real hardware only with NixOS. The Arch Linux, Debian/Ubuntu, Fedora, and generic Linux instructions are based on build dependencies and standard Wayland-session conventions, but direct sessions, portals, and seat access have not been verified on those distributions. Test Mio nested first and keep your existing desktop installed.

## NixOS

Add Mio to your system flake inputs and module list:

```nix
{
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    mio = {
      url = "github:bakumugi777/mio-wm";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs = { nixpkgs, mio, ... }: {
    nixosConfigurations.HOSTNAME = nixpkgs.lib.nixosSystem {
      system = "x86_64-linux";
      modules = [ mio.nixosModules.default ./configuration.nix ];
    };
  };
}
```

Enable Mio and your display manager:

```nix
{
  services.displayManager.sddm.enable = true;
  programs.mio = {
    enable = true;
    xwayland.enable = true;
  };
}
```

`programs.mio.enable` installs Mio, its Wayland session, and the screen-sharing portal integration. Mio updates the D-Bus environment and portal after its Wayland socket is ready; no portal command is needed in KDL. It also makes an unlocked GNOME Keyring Secrets component available for credentials without installing or starting the GNOME desktop. Mio does not launch every XDG autostart entry.

Foot, Waybar, and Wofi are installed by default. Set `programs.mio.recommendedPackages = false;` to omit them. Set `programs.mio.xwayland.enable = false;` if X11 compatibility is unnecessary.

For a local development tree, use:

```nix
mio.url = "path:/home/user/path/to/mio";
```

Apply the configuration, log out, and select **Mio**. A full reboot is normally unnecessary. Put user configuration at `$XDG_CONFIG_HOME/mio/config.kdl` or `$HOME/.config/mio/config.kdl`; Mio starts with built-in defaults when the file is absent.

Additional session arguments can be set with:

```nix
programs.mio.extraSessionArguments = [ "--config" "/path/to/config.kdl" ];
```

### NixOS module options

| Option | Default | Meaning |
|---|---:|---|
| `programs.mio.enable` | `false` | Install Mio and register its session |
| `programs.mio.package` | Mio flake package | Package launched by the session |
| `programs.mio.extraSessionArguments` | `[]` | Additional compositor arguments |
| `programs.mio.xwayland.enable` | `false` | Install and start xwayland-satellite |
| `programs.mio.xwayland.display` | `":100"` | X display used by the satellite |
| `programs.mio.portal.enable` | `true` | Enable the wlr screenshot/screencast portal |
| `programs.mio.recommendedPackages` | `true` | Install Foot, Waybar, and Wofi |

## Build the flake package

```sh
nix build .#mio
./result/bin/mio-compositor --version
./result/bin/mio-compositor --config ./result/share/mio/config.kdl --check-config
```

The output contains `mio-compositor`, `mioctl`, `mio-session`, an example configuration, and `mio.desktop`.

## NixOS development shell

```sh
nix-shell
cargo build --workspace
```

`shell.nix` supplies Rust and the native libraries required by the winit and DRM/KMS backends. On NixOS, binaries started outside this shell may fail to find libraries such as `libwayland.so`.

## Guix and Guix System

The repository contains a reproducible Guix package definition in `guix.scm`.
It reads the exact crate versions and checksums from `Cargo.lock`, fetches the
pinned Smithay revision as a fixed-output source, and builds Cargo offline.

Build the package from a checkout:

```sh
git clone https://github.com/bakumugi777/mio-wm.git
cd mio-wm
guix build -f guix.scm
```

Install it in the current Guix profile:

```sh
guix package --install-from-file=guix.scm
```

The package contains `mio-compositor`, `mioctl`, `mio-session`, the example
configuration, and `share/wayland-sessions/mio.desktop`. A per-user profile is
enough for command-line or nested use. Display managers usually discover
sessions from a system profile, so Guix System users should include the package
returned by `guix.scm` in `packages` in their operating-system configuration.
Seat access, a display manager, portals, optional `xwayland-satellite`, and
session applications remain system configuration concerns; this package does
not silently enable services.

Mio has not yet been tested on Guix System hardware. Test a nested session first
and keep another working desktop session installed.

## Other Linux distributions

Mio requires Rust 1.85 plus development packages for Wayland, xkbcommon, libinput, libseat, udev, GBM, EGL, OpenGL, DRM, and display-info. These commands are dependency examples, not a statement that Mio has been tested on each distribution.

### Arch Linux

```sh
sudo pacman -S --needed base-devel git rust pkgconf wayland libdrm libxkbcommon \
  libinput seatd systemd-libs mesa libdisplay-info
```

### Debian / Ubuntu

```sh
sudo apt update
sudo apt install build-essential git cargo pkg-config libwayland-dev libdrm-dev \
  libxkbcommon-dev libinput-dev libseat-dev libudev-dev libgbm-dev \
  libegl1-mesa-dev libgl1-mesa-dev libdisplay-info-dev
```

### Fedora

```sh
sudo dnf install gcc gcc-c++ make git cargo pkgconf-pkg-config wayland-devel \
  libdrm-devel libxkbcommon-devel libinput-devel libseat-devel systemd-devel \
  mesa-libgbm-devel mesa-libEGL-devel mesa-libGL-devel libdisplay-info-devel
```

If the packaged Rust is older than 1.85, use rustup or another toolchain satisfying `rust-toolchain.toml`. On other distributions, install packages providing these pkg-config modules:

```text
wayland-server  libdrm  libdisplay-info  gbm  libinput
libseat         libudev xkbcommon        egl  gl
```

The installer never invokes a package manager. Check requirements and install with:

```sh
git clone https://github.com/bakumugi777/mio-wm.git
cd mio-wm
./install.sh check
sudo ./install.sh install
```

By default this builds the locked release workspace and installs:

```text
/usr/local/bin/mio-compositor
/usr/local/bin/mioctl
/usr/local/bin/mio-session
/usr/local/share/mio/config.kdl
/usr/local/share/wayland-sessions/mio.desktop
```

Change the prefix or stage a package with:

```sh
PREFIX="$HOME/.local" ./install.sh install
DESTDIR="$PWD/pkg" PREFIX=/usr ./install.sh install
```

A user prefix needs no root access, but some display managers do not search user session directories before login. Use `/usr` or `/usr/local` for reliable system-wide session discovery.

Uninstall only files recorded in the installation manifest:

```sh
sudo ./install.sh uninstall
PREFIX="$HOME/.local" ./install.sh uninstall
```

Use the same `PREFIX` and `DESTDIR` as installation. The installer never changes the user's `config.kdl`.

## Desktop sessions outside NixOS

`install.sh` installs a standard Wayland session entry usable by SDDM, GDM, greetd-based greeters, and other display managers that search that location. Distribution packages must provide seat access, portals, optional xwayland-satellite, and session applications such as a terminal, bar, launcher, notification daemon, IME, and lock screen. Test [nested startup](getting-started.md) first.

The direct backend needs a login session with DRM and input access through logind or seatd. Screen sharing and OBS require `xdg-desktop-portal` plus `xdg-desktop-portal-wlr`; X11 applications require `xwayland-satellite`.
