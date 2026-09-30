# Getting Started

[日本語版](getting-started-jp.md)

Start Mio nested inside your current Wayland desktop. This lets you test its controls and configuration while retaining a working session to return to.

For a NixOS display-manager session, first enable the module described in [Installation](installation.md). After testing nested mode, log out and select **Mio** from the session list.

## 1. Build and validate the configuration

On NixOS, run these commands from the repository root:

```sh
nix-shell
cargo build --workspace
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

On another distribution, install the dependencies listed in [Installation](installation.md), then run the two Cargo commands. If Mio was installed with `install.sh`, validate the standard configuration path with:

```sh
mio-compositor --check-config
```

To use the repository example as your user configuration, first make sure it will not overwrite an existing file:

```sh
mkdir -p "${XDG_CONFIG_HOME:-$HOME/.config}/mio"
cp config/mio.kdl "${XDG_CONFIG_HOME:-$HOME/.config}/mio/config.kdl"
```

## 2. Start a nested session

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info \
cargo run -p mio-compositor -- \
  --config config/mio.kdl \
  --command foot
```

A host Window containing Mio and `foot` should appear. The default `spawn-at-startup` programs, such as Waybar, also start. If the host desktop captures a shortcut first, use mouse controls or a temporary non-conflicting binding.

With installed binaries:

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info \
mio-compositor --command foot
```

## 3. Check the basic controls

- `Super+Q`: close the focused Window
- `Super+Arrow` or `Super+H/J/K/L`: move Focus
- `Super+Ctrl+Arrow`: move the Camera by one viewport
- `Super+1` through `Super+0`: change Camera zoom
- `Super+V`: toggle Overview
- `Super+R`: reload KDL configuration

See the [configuration reference](configuration.md) for all controls.

## 4. Exit normally

From a terminal inside Mio:

```sh
cargo run -p mio-compositor --bin mioctl -- quit
```

Use `mioctl quit` with installed binaries. Force-closing the terminal or host Window can make clients report `Broken pipe`; use IPC for normal shutdown.

## 5. Direct backend

After testing nested mode, switch to a text VT and run:

```sh
nix-shell
RUST_LOG=info cargo run -p mio-compositor -- \
  --backend udev \
  --config config/mio.kdl \
  --command foot
```

Do not launch the direct backend from an existing graphical session. Mio currently targets single-GPU, single-connected-Output setups for primary testing. Keep another VT available so you can inspect or stop the process if necessary.

For a system-wide installation, normally select Mio from the display manager instead. Keep your existing desktop installed, and verify that `mioctl quit` returns you to the display manager.
