# 澪 (Mio)
  <p align="center">
    <img src="./mio-logo.png" alt="Mio logo" width="320">
  </p>


https://github.com/user-attachments/assets/6c5abf94-06e9-474b-b595-3d0bda80ea90


[日本語版](readme-jp.md)

**Instead of switching workspaces, look across one world that extends without end.**

Mio is a lightweight Wayland compositor and tiling window manager built with Rust and
Smithay. Windows live in one continuous two-dimensional World rather than fixed
workspaces or a separate overview layout. The display is a Camera looking into that
World, and panning or zooming changes how you view the same space. Mio is designed to
be comfortable with either a keyboard or a mouse.

## Highlights

- **One continuous 2D World:** place windows in every direction without workspace boundaries
- **Camera-based navigation and overview:** smoothly pan and zoom without rebuilding the layout
- **Grid-based tiling and floating in the same World:** no separate window-management model
- **Comfortable mouse control:** move and resize windows, pan and zoom the Camera, and
  change focus using screen edges and button chords
- **Water-inspired visuals:** optional focus glow, window transitions, and cursor wake
- **One Action model:** keyboard, mouse, and IPC use the same operations
- **External control with `mioctl`:** script windows, the Camera, properties, and logout
- **Practical Wayland integration:** fcitx5, clipboard, drag and drop, session lock,
  portal-based screenshots and OBS capture, and xwayland-satellite
- **Declarative KDL configuration:** configure window rules, appearance, effects,
  key bindings, mouse gestures, and startup commands in one format
- **Low idle memory use:** in the author's environment, Mio itself uses 100 MB of RAM
  when idle

Mio does not have traditional workspaces. Windows exist in one continuous 2D World,
and the screen is treated as a Camera looking into that World.

Start with [Installation](docs/installation.md) and
[Getting Started](docs/getting-started.md).

> [!IMPORTANT]
> Mio is a personal project built for the author's own desktop environment. Other
> people may use this public repository, but Mio is not offered as a general-purpose
> product with guaranteed support, and **feature requests are not accepted**. Bug
> reports with reproducible steps and patches are considered according to the author's
> needs and Mio's design principles.

> [!NOTE]
> Generative AI has been used for implementation, research, testing, and documentation.
> The author remains responsible for design decisions, acceptance, hardware testing,
> and publication.

Mio is available under the [MIT License](LICENSE). Fork it if you need different behavior.

> [!WARNING]
> NixOS is currently the only distribution tested on real hardware. Instructions are
> provided for building from source on Arch Linux, Debian/Ubuntu, Fedora, and other
> distributions, but the complete session, portal, and seat-access setup has not been
> tested there.
>
> Multi-monitor support is experimental. Mio currently provides virtual outputs for
> nested testing and an experimental DRM/KMS backend centered on a single output.
> Hotplug behavior, mixed resolutions and scales, and production reliability are not
> yet complete. Use one physical output for normal testing.

## Installation

The supported entry points are listed below. See
[Installation](docs/installation.md) for dependencies, session registration, and
uninstallation.

| Environment | Recommended method |
|---|---|
| NixOS | Flake input and `mio.nixosModules.default` |
| Nix / NixOS trial | `nix build .#mio` or `nix develop` |
| Arch Linux | Install build dependencies with pacman, then use `install.sh` |
| Debian / Ubuntu | Install build dependencies with apt, then use `install.sh` |
| Fedora | Install build dependencies with dnf, then use `install.sh` |
| Other Linux | Provide Rust 1.85 and the required pkg-config modules, then use `install.sh` |

The non-NixOS paths are untested instructions based on the required packages and the
standard Wayland session layout.

```sh
git clone https://github.com/bakumugi777/mio-wm.git
cd mio-wm
./install.sh check
sudo ./install.sh install
```

A system-wide installation also installs a standard Wayland session entry. Log out and
select `Mio` in SDDM or another display manager. Keep your existing desktop installed
until you have confirmed that you can log out of Mio successfully.

## Build requirements

- Rust 1.85, as selected by `rust-toolchain.toml`
- Git, because Cargo fetches the pinned Smithay revision
- Native libraries for Wayland, xkbcommon, libinput, libseat, udev, GBM, EGL, and OpenGL

On NixOS, use the development shell included in the repository:

```sh
nix-shell
```

The repository also provides a flake package and NixOS module for a normal NixOS
session. See [Installation](docs/installation.md), including the SDDM example. The
NixOS module starts the GNOME Keyring Secrets component inside the Mio session, allowing
tools such as GitHub CLI to use credentials stored in the login keyring without the
GNOME desktop. On other distributions, `install.sh` performs the release build,
installs the Wayland session entry, and records a manifest for uninstallation.

## Nested development session

For normal development, use the winit backend to run Mio as a window inside an existing
Wayland session. `winit` is the default, so `--backend` may be omitted.

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info cargo run -p mio-compositor -- \
  --config config/mio.kdl \
  --command foot
```

List all startup options with:

```sh
cargo run -p mio-compositor -- --help
```

## Direct hardware session

The experimental `udev` backend uses DRM/KMS, GBM, EGL, libinput, and libseat to drive
the display directly. It currently selects one GPU, one connected connector, and that
connector's preferred mode.

Run it from a text VT rather than an existing graphical session. The VT seat normally
needs to be active through logind or seatd.

```sh
nix-shell
RUST_LOG=info cargo run -p mio-compositor -- \
  --backend udev \
  --config config/mio.kdl \
  --command foot
```

VT pause and resume are supported. Multiple GPUs and multiple physical outputs are not
yet supported.

Rounded corners, shadows, borders, focus glow, window transitions, and the cursor wake
are available on the direct backend. When a DRM cursor plane is available, the cursor
wake distorts only the completed desktop image and not the cursor itself.

Both standard `ext-image-copy-capture-v1` and compatibility
`zwlr_screencopy_manager_v1` are available on the nested and direct backends. Sandboxed
applications capture through a portal picker. Non-sandboxed processes that can connect
directly to Mio's regular Wayland socket are treated as part of the trusted desktop
session and may access capture protocols directly. Capture is rejected while the
session is locked.

On the GPU selected at startup, Mio rescans and recovers from disconnects, reconnects,
and mode-list changes. It can start with no connected output and wait for one.
`--command` and `spawn-at-startup` clients are deferred until the first real output is
available. The cursor uses a DRM cursor plane when possible and falls back to normal
composition when no plane is available or the image is too large. During an
`ext-session-lock-v1` lock, Mio renders only an opaque safety frame, lock surfaces, and
the interaction cursor.

Use the winit backend for normal development until the direct backend is complete.

## Startup programs

Programs that should run each time Mio starts can be declared in KDL:

```kdl
spawn-at-startup "waybar"
spawn-at-startup "kaname" "--applications"
```

Each value is passed as one argument, not as a shell command string. Commands run once
after Mio's Wayland and IPC sockets are ready; configuration reload does not run them
again.

Child processes receive `WAYLAND_DISPLAY`, `MIO_SOCKET`,
`XDG_CURRENT_DESKTOP=mio`, `XDG_SESSION_DESKTOP=mio`, and either
`MIO_BACKEND=winit` or `MIO_BACKEND=udev`. When xwayland-satellite is enabled, they also
receive the compatibility `DISPLAY`.

## Screen recording with OBS

Install `xdg-desktop-portal` and `xdg-desktop-portal-wlr`. A reference NixOS
configuration is available under `memo/nix/configuration.nix`. The portal systemd user
units do not exist until that configuration has been applied and NixOS rebuilt.

At startup, `config/mio.kdl` runs `dbus-update-activation-environment` to publish Mio's
Wayland socket and desktop name to D-Bus/systemd-activated services. The direct backend
also reconnects the wlr and desktop portals in order; the nested backend does not
restart the host portal. NixOS and similar environments must activate
`graphical-session.target`. In OBS, select the same Screen Capture source used with
niri. Source names vary by OBS version and translation. This path prefers Smithay's
standard `ext-image-copy-capture-v1`; legacy `wlr-screencopy` remains for tools such as
grim.

## Configuration

The example configuration is [config/mio.kdl](config/mio.kdl). The standard locations
are listed below. See the [configuration reference](docs/configuration.md) for every
option and key-binding Action. `appearance.opacity` sets the default for all windows,
and later `window-rule` entries can override it per application—for example, making all
windows translucent except `foot`.

```text
$XDG_CONFIG_HOME/mio/config.kdl
$HOME/.config/mio/config.kdl
```

Validate an explicit file without starting the compositor:

```sh
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

If reload fails, Mio keeps the last valid configuration and displays the error. If the
startup configuration is invalid, Mio starts with built-in defaults so it can recover
after the file is fixed and reloaded.

### Output scale

If text, UI, and the pointer appear too small, change the Wayland output scale. The
default is `1.0`; fractional values from `0.5` through `4.0` are supported.

```kdl
output {
    scale 1.25
}
```

This scale is advertised to clients. It is independent from Camera zoom, which changes
how the World is viewed. Reload with `Super+R` or restart Mio after saving.

### Splitting the configuration

`include` loads another KDL file at the point where it appears. Relative paths are
resolved from the including file, and included files may include more files.

```kdl
include "wallpaper.kdl"
```

A missing target is ignored so tools such as wallpaper selectors can generate optional
configuration later. An existing unreadable or invalid file, or an include cycle, is a
configuration error. Includes are evaluated again on reload, but `spawn-at-startup`
runs only when Mio starts.

```kdl
// wallpaper.kdl
spawn-at-startup "mpvpaper" "*" "/path/to/wallpaper.mp4"
```

### Key bindings

Join `Ctrl`, `Alt`, `Shift`, and `Super` with a key name using `+`. Keys may be any
single character or an xkbcommon keysym name.

```kdl
bind "Super+Space" "spawn" "wofi" "--show" "drun"
bind "PrintScreen" "spawn" "grim"
bind "Super+F12" "close"
bind "XF86AudioRaiseVolume" "spawn" "wpctl" "set-volume" "@DEFAULT_AUDIO_SINK@" "5%+"
```

Common supported names include:

- `Space`, `Enter`, `Tab`, `Escape`, and `BackSpace`
- `Left`, `Right`, `Up`, `Down`, `Home`, `End`, `PageUp`, and `PageDown`
- `Insert`, `Delete`, `PrintScreen`, and `Menu`
- `F1` through `F35`
- keypad names such as `KP_0`, `KP_Enter`, and `KP_Add`
- media keys such as `XF86AudioRaiseVolume`, `XF86AudioMute`, and
  `XF86MonBrightnessUp`
- any other keysym recognized by xkbcommon

Aliases such as `Esc`, `SpaceBar`, `PrtSc`, `PrtScr`, `PgUp`, and `PgDn` are accepted.
Character keys are matched against the keymap's base key rather than the shifted
symbol, so write `Shift+1`, not `!`.

External commands are passed directly as a program and separate arguments. Explicitly
use `sh -c` or `bash -c` only when pipes, redirects, environment expansion, or another
shell feature is required.

```kdl
bind "Super+W" "spawn" "sh" "-c" "my-command | another-command"
```

Declaring any `bind` replaces the complete built-in key map. Declare every default you
want to retain. Duplicate chords are configuration errors.

## Basic controls

Mio treats keyboard and mouse input as equal control methods. Both are configurable,
and one click gesture may compose multiple Actions.

### Keyboard

Default bindings use Super as a common base and add only Shift or Ctrl for each group.

| Key | Action |
|---|---|
| `Super+Arrow` / `Super+H/J/K/L` | Focus in that World direction and reveal it with the Camera if needed |
| `Super+Ctrl+Arrow` / `Super+Ctrl+H/J/K/L` | Move the Camera one screen left/down/up/right |
| `Super+Ctrl+Shift+H/J/K/L` | Move the Camera one Grid cell left/down/up/right |
| `Super+1`–`Super+9` | Set absolute Camera zoom from 0.1 through 0.9 |
| `Super+0` | Restore maximum Camera zoom, 1.0 |
| `Super+Shift+Arrow` | Move the focused window by one Grid cell |
| `Super+Ctrl+Shift+Arrow` | Resize the focused window by one Grid cell |
| `Super+Shift+H/J/K/L` | Place the next window left/down/up/right |
| `Super+N` | Switch the active output Camera |
| `Super+F` | Toggle tiled/floating |
| `Super+Enter` | Toggle fullscreen |
| `Super+M` | Toggle maximized |
| `Super+Z` | Toggle initial/half window width |
| `Super+Q` | Close the focused window |
| `Super+O` | Toggle opacity between 1.0 and 0.8 |
| `Super+Shift+O` | Clear the runtime opacity override |
| `Super+V` | Toggle overview |
| `Super+S` | Select the focused window and restore normal zoom |
| `Super+B` | Toggle blur for the focused window |
| `Super+W` | Toggle the cursor wake |
| `Super+R` | Reload the configuration |

### Mouse

| Gesture | Action |
|---|---|
| Right drag | Smoothly pan the Camera |
| Hold right and scroll | Smoothly zoom the Camera |
| Hold right and left-drag a window | Smoothly move the window |
| Left-drag a window border or outer edge | Resize; commit to the Grid on release |
| Double right-click a window | Toggle initial/half window width |
| Hold right and middle-click a window | Toggle tiled/floating |
| Hold right and left-click a window | Center the Camera on it and restore zoom 1.0 |
| Hold right and triple left-click a window | Close the window |
| Scroll at an output edge | Move focus forward or backward along that edge's axis |
| Middle-click at an output edge | Set where the next window will be placed |
| Short right-click at an output edge | Run that edge's `edge-command` |

Mouse gestures are configured with `mouse-bind` in
[config/mio.kdl](config/mio.kdl). Like key bindings, they map an input gesture to one or
more Actions, and only declared bindings are enabled. The `mouse` block contains global
settings such as the pointer hide delay, not gestures.

For example, bind `toggle-window-size` to a key with
`bind "Super+Z" "toggle-window-size"`, or set a zoom with
`bind "Super+5" "camera-zoom" 0.5`. A `right+left-click` binding may contain child
Actions `camera-center` and `camera-zoom 1.0`; Mio applies them in declaration order.
There are no implicit Actions. Multi-button gestures are matched in KDL declaration
order. See [docs/spec.md](docs/spec.md) for matching rules and Action semantics.

## IPC and mioctl

Terminals launched inside Mio inherit `MIO_SOCKET`, allowing `mioctl` to control that
Mio instance. See the [IPC reference](docs/ipc.md) for every command, JSON response,
and exit status. Compositor startup options are documented in the
[CLI reference](docs/cli.md).

```sh
cargo run -p mio-compositor --bin mioctl -- focused-window
cargo run -p mio-compositor --bin mioctl -- camera
cargo run -p mio-compositor --bin mioctl -- windows
cargo run -p mio-compositor --bin mioctl -- outputs
cargo run -p mio-compositor --bin mioctl -- state
```

Main mutating commands are shown below. Replace `ID` with a window ID from `windows` or
`state`.

```sh
cargo run -p mio-compositor --bin mioctl -- focus ID
cargo run -p mio-compositor --bin mioctl -- camera-to ID
cargo run -p mio-compositor --bin mioctl -- move-window ID left
cargo run -p mio-compositor --bin mioctl -- resize-window ID down
cargo run -p mio-compositor --bin mioctl -- toggle-floating ID
cargo run -p mio-compositor --bin mioctl -- set-opacity ID 0.5
cargo run -p mio-compositor --bin mioctl -- clear-opacity ID
cargo run -p mio-compositor --bin mioctl -- set-property ID blur true
cargo run -p mio-compositor --bin mioctl -- clear-property ID blur
cargo run -p mio-compositor --bin mioctl -- close ID
```

Log out cleanly from a terminal launched inside Mio:

```sh
cargo run -p mio-compositor --bin mioctl -- quit
```

When installed, use `mioctl quit`. The request passes through the normal event-loop
shutdown, including xwayland-satellite cleanup and IPC socket removal. From outside
Mio, pass the path printed by `Mio IPC ready` with `--socket PATH`. Only the owning user
can access the socket.

## External tool integration

External tools may obtain Mio's `state` snapshot and invoke shared Actions such as
`mioctl focus ID`. This is a generic boundary rather than an API tied to one tool. Mio,
Shirube, and Kaname do not require one another.

## Experimental virtual outputs

A nested window can be split horizontally to display two Cameras at once:

```sh
WINIT_UNIX_BACKEND=wayland RUST_LOG=info cargo run -p mio-compositor -- \
  --virtual-outputs 2 \
  --command foot
```

Click either half to make that Camera active, or use `Super+N`. This is a development
feature and does not imply complete physical multi-monitor support.

## X11 compatibility

Install `xwayland-satellite` when needed and start Mio with:

```sh
cargo run -p mio-compositor -- --xwayland-satellite --command foot
```

The default X display is `:100`; change it with `--xwayland-display :NUMBER` if already
in use. Native Wayland clients continue working if the satellite fails to start.

## Input methods

Mio advertises the text-input, input-method, and virtual-keyboard Wayland protocols.
The session is responsible for starting and configuring an IME such as fcitx5.
Candidate windows are regular popups constrained to the output. Preedit, commit,
candidate selection, following moved windows, and recovery after lock/unlock have been
tested with fcitx5 in a Mio desktop session.

## Development checks

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cargo build --workspace
```

Run only the pure World logic tests with:

```sh
cargo test -p mio-core
```

Enable five-second rendering diagnostics with:

```sh
WINIT_UNIX_BACKEND=wayland \
RUST_LOG=info,mio_compositor::diagnostics=debug \
cargo run -p mio-compositor -- --config config/mio.kdl --command foot
```

## Documentation

- [docs/requirements.md](docs/requirements.md): requirements and implementation phases
- [docs/installation.md](docs/installation.md): build and installation methods and limitations
- [docs/getting-started.md](docs/getting-started.md): shortest path from nested startup to logout
- [docs/configuration.md](docs/configuration.md): public KDL and key-binding Action reference
- [docs/keybindings.md](docs/keybindings.md): default bindings and customization
- [docs/camera.md](docs/camera.md): Camera Actions, following, zoom, and multiple outputs
- [docs/overview.md](docs/overview.md): overview behavior and its relationship to the World
- [docs/window-properties.md](docs/window-properties.md): rule composition and Property precedence
- [docs/cli.md](docs/cli.md): `mio-compositor` command-line options
- [docs/ipc.md](docs/ipc.md): `mioctl`, IPC commands, and JSON responses
- [docs/troubleshooting.md](docs/troubleshooting.md): startup, configuration, and capture diagnostics
- [docs/architecture-overview.md](docs/architecture-overview.md): Japanese architecture overview
- [docs/release-readiness.md](docs/release-readiness.md): 1.0 audit and remaining work
- [docs/spec.md](docs/spec.md): detailed specification
- [docs/architecture.md](docs/architecture.md): component boundaries and design
- [docs/smithay-notes.md](docs/smithay-notes.md): Smithay API research notes
- [docs/phases.md](docs/phases.md): phase-by-phase progress

Mio's core idea is summarized in one sentence:

> The world is one. Windows live in it. The display is only a camera looking into that world.

## License and development method

Mio is available under the [MIT License](LICENSE). Generative AI has been used in the
implementation and documentation. When changing Mio, do not assume AI output is
correct; verify it against `AGENTS.md`, the requirements, tests, and real behavior.
