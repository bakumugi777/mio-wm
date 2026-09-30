# Mio IPC Reference

[日本語版](ipc-jp.md)

## Connection

Mio creates an instance-specific Unix socket at `$XDG_RUNTIME_DIR/mio-WAYLAND_SOCKET.sock` with owner-only `0600` permissions. Child processes launched by Mio receive its actual path through `MIO_SOCKET`.

`mioctl` connects to `MIO_SOCKET` by default. Specify the socket when controlling Mio from outside its environment:

```sh
mioctl --socket /run/user/1000/mio-wayland-2.sock state
```

A request is one UTF-8 command line, read through the first newline or EOF, with a 4096-byte limit. A response is one JSON object; a trailing newline is not required by the protocol.

## mioctl exit status

- `0`: help/version was displayed, or Mio returned `{"ok":true,...}`
- nonzero: argument, connection, or communication failure, or Mio returned `{"ok":false,...}`

Successful responses go to standard output. CLI errors and rejection reasons from Mio go to standard error.

## Read commands

| Command | Main response fields |
|---|---|
| `state` | `windows`, `focused_window`, `camera`, `outputs`, and `cameras` in one snapshot |
| `windows` | `windows` |
| `focused-window` | `window`, or `null` if no Window is focused |
| `camera` | active Camera in `camera` |
| `outputs` | adapter Outputs in `outputs` and Core Cameras in `cameras` |

A Window object has these fields:

| Field | Meaning |
|---|---|
| `id` | Window ID within this instance |
| `app_id`, `title` | xdg-toplevel metadata, or `null` when absent |
| `rect` | `x`, `y`, `width`, and `height` on the World Grid |
| `focused` | whether the Window has Focus |
| `presentation` | `normal`, `maximized`, or `fullscreen` |
| `opacity`, `floating`, `blur` | effective Window Properties |

A Camera object contains `output_id`, `x`, `y`, and `zoom`. An Output object contains `id`, `name`, logical geometry (`x`, `y`, `width`, `height`), and `scale`.

## Action commands

`DIR` is `left`, `right`, `up`, or `down`. `ID` is an unsigned integer obtained from a read command.

| Command | Meaning |
|---|---|
| `activate-output ID` | Change the active Output Camera |
| `focus ID` | Focus a Window |
| `camera-to ID` | Move the Camera to the Window center |
| `camera-step DIR` | Move the active Camera by one viewport |
| `move-window ID DIR` | Move a Window by one Grid cell |
| `resize-window ID DIR` | Resize a Window by one Grid cell |
| `toggle-floating ID` | Toggle the Window's floating Property |
| `close ID` | Request that the Window close |
| `set-property ID opacity FLOAT` | Set a runtime opacity override |
| `set-property ID floating BOOL` | Set a runtime floating override |
| `set-property ID blur BOOL` | Set a runtime blur override |
| `clear-property ID PROPERTY` | Clear the selected runtime override |

`PROPERTY` is `opacity`, `floating`, or `blur`; `BOOL` is `true` or `false`. `set-opacity ID FLOAT` and `clear-opacity ID` remain as compatibility aliases. New integrations should use `set-property` and `clear-property`.

Action commands use the same Core Action path as keyboard and mouse input. IPC does not maintain separate Window state.

## Lifecycle command

`quit` is a compositor lifecycle command rather than an `Action`. Mio begins normal shutdown after returning `{"ok":true}`.

## Responses

A successful response always has `ok` set to `true`:

```json
{"ok":true}
```

On failure, `ok` is `false` and `error` contains a human-readable reason:

```json
{"ok":false,"error":"unknown window 9"}
```

The current IPC is intended for short-lived local CLI connections. It does not provide subscriptions, event streams, long-lived clients, or remote transport.
