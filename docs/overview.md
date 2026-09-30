# Overview

[日本語版](overview-jp.md)

Mio's Overview is not a separate screen or window list. It is the same World viewed from farther away through the Camera. Window World coordinates, sizes, relative positions, and identities do not change.

## Controls

The default configuration provides these controls:

| Key | Action |
|---|---|
| `Super+V` | Toggle between the normal zoom `1.0` and Overview zoom `0.35` |
| `Super+S` | Center the focused Window and return to zoom `1.0` |
| `Super+Arrow` / `Super+H/J/K/L` | Move Focus to another Window in the World |
| `Super+Ctrl+Arrow` | Move the Camera by one viewport |
| `Super+1` through `Super+0` | Select an absolute zoom directly, not only in Overview |

Focus movement in Overview uses the same Windows and Actions as normal operation. There are no duplicate Windows or separate selection list. `select-overview` centers the currently focused Window and restores the normal zoom, so first choose the target with directional Focus.

## Relationship to placement

Overview is only a display transform; it does not expand the search area for placing new Windows. Even while zoomed out, placement uses the normal viewport, rather than automatically filling every visible empty area in the distant view.

Windows, gaps, popups, and subsurfaces are reduced with the same presentation scale. This preserves the relationship between apparent distance and occupied World space; Mio does not create an Overview-specific layout.

See the [Camera reference](camera.md) for the lower-level Camera Actions.
