# Troubleshooting

[日本語版](troubleshooting-jp.md)

## `cargo: command not found`

On NixOS, enter `nix-shell` from the repository root first. It also configures Mio's native-library paths.

## Shared-library errors such as `could not load libwayland.so`

You probably started `target/debug/mio-compositor` outside the Nix development shell. Enter `nix-shell` in the repository, including when using another TTY.

## `failed to initialize Mio`, a white screen, or extreme slowness

- use `WINIT_UNIX_BACKEND=wayland` for nested testing
- launch the direct backend from a text VT with `--backend udev`, not from a graphical session
- confirm that you built and started the new binary
- use `RUST_LOG=info` and inspect the selected backend and GPU renderer

Nested and direct modes have different startup requirements. Adding `WINIT_UNIX_BACKEND` to a direct-backend command does not fix direct-backend problems.

## Configuration changes are not applied

```sh
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

While Mio is running, invoke `reload-config` (`Super+R` by default). A failed reload preserves the last valid configuration and reports the cause on screen and in the log. Also check `configuration loaded path=...` in the startup log to ensure Mio is reading the expected path.

## Child applications report `Broken pipe`

Force-stopping the compositor closes its Wayland socket, so Foot and GTK applications may report `Broken pipe`. Unless Mio crashed unexpectedly, this is a consequence rather than a client-side cause. Exit normally with `mioctl quit` from inside Mio.

## OBS capture is black, frozen, or absent

- install `xdg-desktop-portal` and `xdg-desktop-portal-wlr`
- restart the user services or session after changing system configuration
- with the NixOS module, Mio updates the D-Bus environment and reconnects the portal after Outputs are ready; do not add portal startup commands to KDL
- check `systemctl --user status xdg-desktop-portal.service xdg-desktop-portal-wlr.service` after logging into Mio
- remove stale OBS sources and create a new **Screen Capture** source

Mio prefers the standard `ext-image-copy-capture-v1` protocol and exposes legacy `wlr-screencopy` for compatibility. Capture is rejected while the session is locked.

## An X11 application cannot open the display

Install `xwayland-satellite` and start Mio with `--xwayland-satellite`. Only applications launched by Mio inherit the compatibility `DISPLAY`; a shell outside the Mio session does not inherit it automatically.

## IME does not work

Mio exposes the text-input, input-method, and virtual-keyboard protocols. Starting Fcitx5, setting environment variables, and configuring add-ons remain session responsibilities. Nested and full desktop sessions have different environments.

## Detailed logs

```sh
WINIT_UNIX_BACKEND=wayland \
RUST_LOG=info,mio_compositor::diagnostics=debug \
cargo run -p mio-compositor -- --config config/mio.kdl --command foot
```

When diagnosing a problem, retain the startup command, backend, reproduction steps, and relevant log lines.
