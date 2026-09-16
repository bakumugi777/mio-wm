# Configuration

Mio reads declarative KDL from `$XDG_CONFIG_HOME/mio/config.kdl`, falling back to
`$HOME/.config/mio/config.kdl`. If that implicit file is absent, built-in defaults are
used. [`mio.kdl`](mio.kdl) is a complete example.

An explicit path can be selected and checked without starting the compositor:

```sh
cargo run -p mio-compositor -- --config config/mio.kdl --check-config
```

Use the `reload-config` Action to reload the selected file. Parsing and validation
finish before the active configuration is replaced, so an invalid edit leaves the
previous configuration active. `edge-command` is the only configuration entry that
launches an external process. Its values are an executable followed by literal argv;
Mio does not interpret shell operators.

Declaring one or more `bind` nodes replaces the complete built-in binding set. Window
rules perform exact `app-id` and/or `title` matching and support `opacity` and
`floating`. If multiple rules match, later property values win.
`background-color` sets the solid World background visible where no Window or
layer-shell background is drawn.
The focused Window is marked by a thin water-film line along its bottom edge with a
water-like light centered over it. The light width, shared core height, and color are
configurable; setting either dimension to zero disables the complete indicator.
Every Window also has a subtle inner border. `window-border-width` and
`window-border-color` control it; a zero width or transparent color disables it. The border
follows the configured corner radius without changing Window geometry or pointer input.
The default Shadow is centered on the Window edge; an explicit nonzero `shadow-offset` may
expose a strip of the configured background color between the Window and the strongest Shadow.
`animation.speed` controls the shared
Camera, Window geometry, zoom, and opacity interpolation; zero disables animation.
The example binds `toggle-overview` and `select-overview`; Overview is purely a Camera
zoom and does not create another Window representation.
`camera-*` moves by a viewport, while `camera-nudge-*` moves the same Camera by one
World grid cell.
`edge-command "EDGE" "PROGRAM" "ARG"...` assigns a short right-click near the left,
right, top, or bottom Output edge. Declaring any edge command replaces the built-in
set; the built-in default is `edge-command "bottom" "wofi" "--show" "drun"`.
`place-next-left`, `place-next-right`, `place-next-up`, and `place-next-down` select the
direction of the next successfully opened Window only. The selection then returns to
the default direction, right.

`spawn-at-startup "PROGRAM" "ARG"...` starts a command once, after Mio's Wayland and
IPC endpoints are ready. It may be declared multiple times. Every quoted value becomes
one literal argv item; Mio does not invoke a shell. Spawned processes receive Mio's
`WAYLAND_DISPLAY` and `MIO_SOCKET`, plus `DISPLAY` when Xwayland Satellite is enabled.
Configuration reload does not run startup commands again.
