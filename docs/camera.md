# Camera Reference

[日本語版](camera-jp.md)

Mio's display is a Camera looking into one continuous two-dimensional World. Moving or zooming the Camera does not change Window World coordinates, sizes, ownership, or lifetime. Camera boundaries are navigation stops, not workspace or Window-placement boundaries.

## State

The Camera has the following state:

- `position`: the top-left position in the World; fractional and negative values are allowed
- `viewport`: the width and height of the Grid visible at zoom `1.0`
- `zoom`: the absolute zoom; current KDL Actions accept `0.1..=1.0`

`camera { viewport W H }` defines the logical viewport at normal zoom. Reducing the zoom reveals a larger World area without changing the logical viewport or Window geometry. Overview also uses this Camera zoom rather than a separate Window layout.

## Actions

| Action | Meaning |
|---|---|
| `CameraStep(Direction)` | Move by one configured viewport |
| `CameraNudge(Direction)` | Move by one Grid cell |
| `CameraPan { delta_x, delta_y }` | Move by a World-space delta, including fractional values |
| `CameraTo(Window)` | Move to a natural viewport stop containing the Window center |
| `CameraCenter(Window)` | Center the Window without changing zoom |
| `CameraFollow(Window)` | Do nothing if fully visible; otherwise follow only on the necessary axes |
| `CameraZoom(value)` | Set the absolute Camera zoom |

If a Window is entirely outside the view, or larger than the visible area at the current zoom, `CameraFollow` centers it. It does not change zoom.

Focus and Camera are independent Core states and use separate Actions. Input adapters that semantically move Focus, such as keyboard and screen-edge controls, compose `Focus` with `CameraFollow`. Pointer focus does not move the Camera implicitly, and repeated protocol activation of an already focused Window does not repeatedly follow it.

Camera Actions are rejected during fullscreen so the Output view remains fixed. They become available again after leaving fullscreen.

## Multiple Outputs

Each Output has its own Camera. Input Actions target the active Output's Camera, and `cycle-output` changes that target. This does not divide the World or Windows into per-Output containers.
