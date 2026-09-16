# Mio architecture

## Current phase

The repository has entered Phase 14 after completing the initial Phase 10-12 paths.
`mio-core` provides the continuous World and remains
Smithay-independent. Each Window now owns orthogonal `GridConstraint` and `Presentation`
state. `mio-compositor` maps these states to Smithay Space and xdg-toplevel protocol
state without duplicating logical geometry. Declarative KDL is parsed and validated at
the compositor boundary; it cannot introduce Smithay types into Core.

## Dependency direction

```text
Wayland clients
      |
      v
mio-compositor (Smithay adapter, protocols, input, rendering)
      |
      v
mio-core (World, Grid, Window, Camera, Focus, Property, Action)
```

`mio-core` must never depend on Smithay or expose Wayland-, renderer-, DRM-, or
backend-specific types. `mio-compositor` translates external events into Core actions
and translates Core geometry into render coordinates.

## State ownership

Core owns each window's logical world rectangle, camera position, focus, grid
constraint, presentation, and Property precedence. The compositor adapter owns Wayland
objects and derived rendering state. Animation may keep
separate target/render values, but never updates logical world geometry frame by frame.

There are no workspace containers. All windows remain in one world, including floating
windows and windows outside the current camera viewport.

Each Output owns one independent Core Camera. One Output is active at a time for Camera
Actions and new Window placement; activation swaps only the Action target and preserves
every Camera's position, viewport, and zoom. Windows are never assigned to Outputs.
The adapter keeps one derived AnimatedRect per Window and Output Camera, so simultaneous
views interpolate independently without mutating Core geometry.

Grid rectangles are non-empty, half-open rectangles with validated `i64` edges. World
coordinates may be negative. Visibility is derived by intersecting Window rectangles
with the Camera viewport and is never stored as Window lifetime state.

Directional focus considers candidates in the requested World half-plane and ranks
them by center distance, perpendicular deviation, forward distance, then stable Window
ID. This makes the nearest visibly adjacent Window win over a distant Window that
happens to align at a slightly straighter angle. Focus and Camera movement remain
separate operations so callers can compose them explicitly.
Destroying an unfocused Window preserves both states. Destroying the focused Window
clears Core and Wayland Seat focus without moving the Camera or implicitly selecting a
remaining Window.
For a focused xdg-toplevel with a surviving protocol parent, the adapter overrides that
generic behavior through the ordinary `FocusWindow` Action. It restores a closed dialog
to its parent without centering the Camera or introducing dialog ownership into Core.
Nested dialogs keep an adapter-only ordered return list: direct protocol parent,
inherited ancestors, then the Window focused immediately before automatic activation.
Closing selects the first surviving candidate, so a conversion dialog returns to a
still-open chooser and falls through to the main application only if it disappeared.
The adapter also composes the standard dialog hints: xdg-dialog modal state, an
xdg-toplevel parent, or equal non-zero minimum and maximum sizes on both axes applies
the existing per-Window floating runtime override. Size constraints are re-evaluated
after surface commits. Clearing every automatic reason clears that override; no
dialog-specific Core type is introduced. App ID and title are not used as dialog
heuristics because one application may create splash screens and independent toplevels
with the same metadata.
Fixed-size floating surfaces retain their committed client size at normal zoom, follow
Camera zoom in Overview, and remain centered within their World presentation rectangle.
Other surfaces use independent horizontal and
vertical adapter scaling while waiting for a requested buffer size, so an early buffer
does not remain visibly undersized until focus changes.
When automatic floating has a live return-focus candidate, the adapter composes the
existing `MoveWindow` and `CameraCenter` Actions to overlap that Window in the same
World. Focus indication is derived from the final Smithay Space element geometry, so a
small centered surface does not inherit the larger logical presentation rectangle.
The first automatic placement resets that Window's adapter interpolation state. When
the focused automatic Window also corrects the Camera, every Window's presentation
interpolation is reset because Camera motion is rendered through all screen rectangles.
This makes the parent-relative position the first stable frame instead of animating out
to the temporary tiled placement and back. Later user-initiated moves keep normal
animation.
An xdg-toplevel role is kept presentation-pending until its first root surface commit.
Mio can send the required initial concrete configure while preserving the previous
focus, then evaluate parent and size metadata before mapping the Window into Smithay
Space and activating it. Core placement remains the sole logical geometry; the pending
flag only controls when adapter-owned content becomes presentable.
Some clients publish fixed-size constraints only after processing their first activated
configure. If an existing return-focus candidate exists and the first commit is still
unclassified, Mio applies the shared `FocusWindow` Action and protocol activation but
defers Space mapping and Camera centering for one additional commit. This bounded
handshake avoids a provisional Camera trip without creating a dialog mode or relying
on application metadata.

Window and Grid geometry remain integral, while Camera position is continuous World
geometry. This permits exact centering at half-cell coordinates and provides the same
position model needed by later smooth pointer panning. Camera stops are derived
navigation targets rather than restrictions on Camera position.

New Window placement anchors its row to the focused Window's top edge and scans right
for free space. It never derives the row from a recentered Camera while focus exists,
so successive Windows stay aligned even though Camera position changes.

Normal focus and keyboard Window movement compose their focus or movement Action with
`CameraFollow`. Follow derives the visible World extent from the current zoom, preserves
the Camera for a fully visible Window, uses the minimum displacement needed to contain a
partially visible Window, and centers a completely invisible Window. A Window larger
than the current visible extent is centered without changing zoom. Resize does not move
the Camera, preserving deliberate multi-Window compositions.

The Phase 3 render transform divides the current logical output into the Camera's
configurable grid dimensions. For each axis it computes:

```text
screen = (world - camera) * output_pixels / viewport_cells
```

The built-in and distributed configuration uses an 8×8 viewport and an 8×8 initial
Window. This changes Grid granularity without adding a second coordinate unit; users may
configure both values independently through the existing KDL fields.

Smithay Space positions are derived render state. Camera movement only resynchronizes
those positions; it does not mutate Window rectangles. Toplevel destruction removes
both sides of the adapter association, while Camera-hidden windows remain alive in Core
and retain their Wayland identity.

`GridConstraint::Tiled` rejects move results that overlap another Window. Resizing a
tiled Window moves tiled neighbours touching each changed edge by the same Grid delta
in that direction, recursively through touching chains. Adjacency is derived from the
current rectangles for each Action; it is not persistent layout state. The resize and
all follower moves are committed atomically. If only the follower proposal conflicts,
Core retries the target rectangle alone and leaves every follower at its original
position; the Action is rejected when that target-only rectangle also conflicts.
`GridConstraint::Floating` relaxes occupancy while retaining the same World coordinates
and never participates as a resize follower or blocker. Tiled Windows may move and resize
beneath it. Returning to tiled is rejected only while overlap with another tiled Window
remains. No floating workspace or second coordinate system exists.

Window placement derives candidates from the Camera and existing World rectangles. If
the Camera sees no Window, placement starts at its center. Otherwise the focused Window
is the anchor and right is the default direction. A `SetNextPlacement` Action captures
the focused Window plus a one-shot direction; successful placement consumes it. No
layout container or lasting directional mode is introduced.

Fullscreen and maximize are two `Presentation` values using the same mechanism. Core
stores one restore rectangle, replaces the active rectangle with the current Camera
viewport, and restores the original rectangle on exit. The adapter derives xdg
fullscreen/maximized states from this Core value.

## Operations

Keyboard, pointer, IPC, and future Yaldra integration converge on the same Core Action
model. Input adapters must not independently implement equivalent window-management
behavior.

The initial IPC adapter listens on an instance-specific Unix socket below
`XDG_RUNTIME_DIR` and exports its path as `MIO_SOCKET`. Read commands serialize Core
and adapter metadata, while mutating commands parse stable Window IDs and construct the
same `Action` values used by keyboard input. IPC does not own a duplicate World,
Camera, focus, or Property state.
The socket is mode 0600. A request ends at its first newline or EOF, is capped at 4096
bytes, and has a short read timeout so an incomplete local client cannot indefinitely
block the compositor event loop. Responses have the same bounded wait, and each
event-loop dispatch accepts at most four connections before yielding to Wayland input
and rendering; level-triggered readiness processes any remainder later.
The `state` read command serializes Windows, focus, the active Camera, Outputs, and
per-Output Cameras from one immutable view of those existing sources of truth. It does
not introduce an external-shell state cache.

## Configuration

`ConfigManager` is the single source of truth for Phase 5 configuration. It loads an
explicit path or the standard per-user KDL path, and falls back to built-in defaults
only when the implicit file is absent. Key chords are parsed separately from Action
names; input matching selects an Action instead of implementing behavior inside the
configuration layer. Reload parses and validates a replacement before committing it,
so invalid files cannot partially mutate active state.

Global appearance remains adapter-owned presentation configuration. `appearance.gaps`
insets normal presented Window rectangles while preserving their Core World/Grid geometry;
the same inset rectangle drives client configure, rendering, animation, and hit testing.
Maximized and fullscreen presentations bypass this inset. Per-Window `opacity`,
`floating`, and `blur` use one Core Property model with Default, matched Config Rule,
and Runtime Override layers. The adapter derives each Window's render alpha, Grid
constraint, and render z-index from the effective Core value; floating renders and hit-tests above
tiled, while focus orders Windows within the same layer. It does not retain a second
semantic copy. Blur is derived as a Smithay framebuffer-effect render element: the
winit adapter inserts each enabled Window's effect immediately behind that Window's
surface elements. It captures content already drawn behind that point in z-order,
filters the texture through a dual Kawase downsample/upsample pyramid, then composites
the Window normally. This supports overlapping blur Windows without a separate blur
stack. Pyramid textures are renderer-owned cache state; renderer resources never enter
Mio Core. Global blur passes and sampling offset are high-level effects settings, while
whether a Window is blurred remains subject to Property precedence.

The winit GLES adapter clips every surface-tree element against its owning Window's
presented geometry when `appearance.corner-radius` is nonzero. The same texture shader
clips the Window's backdrop effect, so surface and blur cannot expose different corner
shapes. Radius zero bypasses the shader wrapper and retains the ordinary rendering path.
The same adapter emits a procedural rounded-box shadow immediately behind each Window.
Its expanded draw bounds belong only to the render element: they do not change Core
geometry, hit testing, tiling, or Camera visibility. A zero shadow radius omits the
element, keeping Window Management independent from the effect.
Pointer input records only a bounded list of recent output-coordinate motion samples
for the optional cursor wake. Speed thresholding belongs to the effects adapter; the
World and Camera never store a trail. The effects adapter interpolates recent fast-motion
samples into a smooth centerline, derives a normal at each interpolated point, and emits
one left/right vertex pair per point. The renderer connects those pairs as a single GLES
triangle strip and samples the captured output framebuffer only inside that strip. No
fullscreen effect pass and no per-sample circles are part of the final image.
Seat pointer coordinates and client input remain raw, preventing the effect from
introducing interaction latency.
Config Rules
match exact app-id and title metadata. Runtime operations use shared Actions and remain
in memory rather than rewriting handwritten KDL.
Configuration load/reload errors are likewise adapter state. The nested renderer draws
a temporary recovery banner above normal content and removes it after a successful
reload; it is suppressed while the session is locked and never enters Mio Core.
The optional startup `--command` is an external client convenience, not part of Mio's
lifetime. A spawn failure is logged while the compositor continues accepting other
clients. Mio supplies `WAYLAND_DISPLAY`, `MIO_SOCKET`, and an optional satellite
`DISPLAY` on that child command directly instead of mutating the compositor process's
global environment after backend initialization.
SIGINT and SIGTERM enter through calloop before backend initialization and stop the
same event loop used by a nested-window close. Cleanup therefore reaps the optional
Xwayland satellite and removes the instance IPC socket before process exit.
The IPC `quit` lifecycle command stops that same event loop and follows the identical
cleanup path; it is not a World Action.

## Animation

Opening and Action-requested closing share one renderer transition progress value.
The selected water or SF shader interprets that same value; closing runs it in reverse
instead of creating a second lifecycle. Closing first removes the Window from the
Core World, restores focus and layout, and keeps only the adapter-side live surface
mapped while progress runs in reverse; the client close request is sent after it becomes
invisible. Thus animation never mutates Core geometry frame-by-frame. A client that
destroys its surface without a prior Mio close Action currently bypasses the closing
transition because no retained renderer snapshot exists.
The opening surface is mapped immediately so clients continue receiving frame callbacks,
but remains fully transparent until an additional content commit arrives after initial
setup. A 350 ms ceiling prevents single-commit clients from remaining hidden indefinitely.
This avoids presenting placeholder commits without inspecting client pixels or delaying
Core placement.

The compositor owns generic scalar current/target interpolation. Render rectangles are
derived targets from Core Window geometry and the logical Camera; each frame advances
only adapter-side render values. Camera movement, Window movement/resize, and
viewport-size zoom therefore share the same rectangle interpolation. Per-Window alpha
uses the same scalar primitive. A speed of zero snaps current to target and disables
animation without changing the model.

## Overview

Camera owns a positive zoom scalar. The compositor's existing World-to-screen transform
applies it around the Camera viewport center, while Core Window rectangles and client
configured sizes remain unchanged. Smithay rescales each completed surface tree only at
render time. Overview is derived from a non-normal zoom and introduces no alternate
layout or Window copies. Selection composes Focus, `CameraCenter`, and `CameraZoom`
before the shared animation layer interpolates the result. `CameraStep` and
`CameraNudge` change the same Camera position by viewport and grid-cell units.

## Phase boundaries

- Phase 1 provides the current minimal Smithay compositor using the winit backend.
- Phase 2 provides the current pure Rust World model in `mio-core` with unit tests.
- Phase 3 provides the current Smithay surface mapping and camera transform.
- Phase 4 provides the current tiled/floating and presentation state.
- Phase 5 provides the current KDL configuration, validation, safe reload, focused
  Window indicator, opacity, configurable bindings, and basic floating rule.
- Phase 6 provides the current shared Property precedence, app-id/title rules, and
  per-Window runtime opacity overrides.
- Phase 7 provides the current generic geometry and opacity interpolation layer.
- Phase 8 provides the current Camera zoom and composition-based Overview navigation.
- Phase 9 provides right-button drag Camera movement through the shared `CameraPan`
  Action. The adapter converts output pixels to continuous World deltas while Core owns
  the Camera position. Holding RMB converts vertical wheel motion into the existing
  `CameraZoom` Action, clamped to the normal `1.0` maximum. Camera-stop snapping and
  inertia remain deferred. Pressing LMB while RMB is already held changes the adapter
  gesture to Window movement: its temporary pixel offset is renderer state, and release
  converts that offset to a Grid origin passed through the existing `MoveWindow` Action.
  The adapter temporarily raises the dragged Window without changing Core focus.
  Successful release composes `MoveWindow` with `FocusWindow` but deliberately omits
  `CameraFollow`; rejected movement preserves the previous focus and Camera.
  Edge LMB resize converts pointer motion to a `ResizeWindowRect` Action whenever it
  crosses a Grid boundary. Core derives followers for each grabbed edge,
  validates the complete proposal, and commits it atomically. The adapter sends the new
  client size and preserves the old buffer's aspect ratio until a matching commit arrives.
  Cursor override state remains renderer/input-adapter state and never enters the World.
  RMB+MMB resolves to the existing `ToggleFloating` Action. The adapter only owns the
  short-lived chord/release bookkeeping; floating state remains the effective Core
  Property, and a successful chord composes `FocusWindow` without `CameraFollow`.
- Phase 10 currently provides the wlr-layer-shell global, Output layer mapping,
  rendering, pointer hit-testing, on-demand keyboard focus, and popup tracking. Layer
  surfaces remain adapter state rather than World Windows. Other daily-use protocols
  remain deferred.
  Xdg popups for both toplevel and layer roots are constrained to Output geometry in
  client coordinates, compensating for the adapter's current render scale.
  Standard data-device, primary selection, and wlr-data-control share Seat keyboard
  focus and remain entirely within the Smithay adapter.
  Data-device drag icons are temporary adapter-owned surface references rendered with
  pointer overlays; they never acquire World Window identity or geometry.
  Layer-shell exclusive zones alter only the adapter's Output render area. Normal and
  maximized Windows use the non-exclusive area, fullscreen uses the full Output, and
  neither case mutates Camera viewport dimensions or World rectangles.
  During fullscreen, Top layer surfaces are retained as adapter state but temporarily
  removed from the Output LayerMap; they are mapped and arranged again on exit. Overlay
  surfaces remain mapped. Per-Output fullscreen visibility is deferred with Phase 14.
  Idle inhibitors are adapter-owned references to Wayland surfaces, including multiple
  inhibitors for one surface. Mio has no idle timeout/DPMS policy yet, so Core receives
  no idle-specific state.
  Viewporter state is protocol/renderer metadata in the adapter. Buffer crop and scale
  requests do not alter Mio World geometry or Camera zoom.
  Fractional-scale preference is likewise derived from Smithay Output scale and kept
  separate from both Camera zoom and the integral World Grid.
  Linux DMA-BUF formats and v4 feedback are derived from the nested GLES renderer.
  Imports are queued by protocol dispatch and completed by that renderer on the next
  redraw before success is reported to the client; no buffer or renderer state enters
  Mio Core.
  Single-pixel buffers enter the same Smithay surface renderer as SHM and DMA-BUF
  buffers and require no Mio-specific Window representation.
  Client alpha-modifier state is consumed inside that surface renderer and composes
  with, but never mutates, Mio's Window opacity Property.
  XDG toplevel icons remain committed Wayland surface metadata. Mio advertises useful
  sizes but does not duplicate icon data in Core merely for a future shell consumer.
  Content-type hints are likewise committed surface metadata. The nested backend
  retains them without pretending to implement a physical Output optimization policy.
  Cross-client xdg-foreign parent relationships remain xdg-shell surface state and do
  not introduce ownership or containment between Core Windows.
  Legacy client cursor surfaces are adapter-owned surface trees rendered in a final
  pass at the Seat pointer position. They never become World Windows or Camera state.
  XDG Activation accepts only tokens tied to Mio's Seat and a current input serial,
  expires them after ten seconds, consumes them on use, and routes accepted requests
  through the ordinary Focus and Camera path.
  Relative pointer and pointer constraints remain adapter-only input state. Locks keep
  logical pointer focus and position fixed while relative events continue; confinement
  rejects movement outside the focused surface or committed region. The nested winit
  backend derives these deltas from absolute host cursor positions, while a future
  libinput backend can provide true raw motion.
  Session lock hides normal content, cancels pre-lock pointer/keyboard/touch grabs,
  and confirms only after an opaque lock frame is submitted; lock surfaces then
  exclusively receive input.
  XDG Dialog modal hints translate to the existing per-Window runtime floating
  Property Action and clear back to Config/Default when unset. Dialogs remain ordinary
  World Windows; no dialog layout or coordinate model is introduced.
- Phase 11 provides explicit xwayland-satellite lifecycle and scoped DISPLAY inheritance.
  Startup clients receive the satellite display only while integration initialized
  successfully; otherwise the host compositor's X display is removed from that child.
- Phase 12 provides the initial Unix-socket read and Action IPC plus `mioctl`.
- Phase 13 defines external application integration as atomic IPC snapshots composed
  with ordinary Actions. An optional Kaname dynamic-provider adapter demonstrates this
  generic boundary; it is not a Kaname-specific Mio API. Shirube remains a status bar
  without a Window list, and no external application is a required dependency. Each
  Window entry carries its focus state from the same snapshot rather than requiring a
  second client-side query.
- Phase 14 currently provides the Core multi-Output Camera model, an active-Output
  Action, IPC inspection/switching, and simultaneous horizontally split virtual
  Cameras in the nested backend. The adapter derives and crops a render view for each
  Camera without duplicating Core Windows. Each nested region now has a real wl_output
  global, mode, logical position, layer map, frame notifications, and screencopy
  framebuffer origin. The direct single-GPU/single-Output adapter provides DRM/KMS,
  libinput, VT lifecycle, connector hotplug, and startup without a connected connector;
  multiple physical Outputs remain deferred pending the Phase 14 policy decisions.

Later features remain deferred according to [phases.md](phases.md).
