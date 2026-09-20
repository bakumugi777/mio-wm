# Smithay implementation notes

## Pinned source

- Upstream: `https://github.com/Smithay/smithay.git`
- Release: `v0.7.0`
- Commit: `f217f62bbe3f5c414997b91d1fe9caeb5e8662d3`
- Upstream commit date: 2026-03-09
- Smithay MSRV: Rust 1.85

Cargo uses the full Git commit in `rev`, rather than following a branch or a moving
version range.

## Backdrop blur sampling

Mio's dual Kawase passes derive `half_pixel` from the input texture for both
downsampling and upsampling. Using the destination size while downsampling doubles
the effective source sampling distance at every level, which makes multi-pass blur
visibly coarse. The configured offset therefore has one meaning across the whole
filter chain, while the framebuffer-effect element continues to follow Smithay's
normal render order and damage tracking.

## `Space::map_element` and stacking

At the pinned Smithay revision, calling `Space::map_element` for an element that is
already mapped removes and reinserts it. Location-only animation updates can therefore
change the order within a shared z-index. Mio reapplies its focus-derived raise after
the per-frame remaps so the focused Window remains topmost and `element_under` uses
the same order as rendering. This remains presentation state in the adapter; Core does
not gain a second Window stack.

## Render diagnostics and damage

`desktop::space::render_output` returns `RenderOutputResult::damage` from its
`OutputDamageTracker`. Mio uses those physical rectangles only for optional diagnostic
aggregation; they remain owned by Smithay and are not copied into compositor state.
The value describes base-scene damage and does not account for Mio's later overlay and
framebuffer-effect passes, so diagnostics label it accordingly. CPU wall time around
the redraw path is also not a GPU timestamp.

At the pinned revision, passing zero as the buffer age means the backbuffer has no
reusable history, so Smithay intentionally damages the whole Output. Mio does this for
the nested winit backend because Mesa's Wayland EGL surface can intermittently return
`EGL_BAD_SURFACE` from `WinitGraphicsBackend::buffer_age()` during ordinary client
commits, not only during a host-window resize. Reusing uncertain history can also leave
stale pixels underneath translucent layer-shell content. Event-driven redraw still
prevents idle rendering, and Mio does not swap when Smithay reports no current scene
damage. A future DRM backend may use reliable per-output buffer age normally.

Closing snapshots, cursor wake, surface cursors, and drag icons may require submission outside
ordinary scene damage. The direct backend represents the configuration-error overlay as solid
render elements in its DRM scene. Their render-element IDs remain stable while the output size
and error text are unchanged, and their commit counter advances only when either value changes.
Otherwise `OutputDamageTracker` treats an unchanged warning as new damage every frame and keeps
the DRM submission loop active. The nested backend draws the warning into the bound framebuffer
after the ordinary scene; showing the warning must not itself request another redraw.

The nested winit backend must not request another redraw unconditionally. Mio wakes it
from a calloop channel on Wayland surface commits, requests immediately after host
input, and chains redraws only while a submitted frame, animation, cursor wake, or
pending click timeout still needs progress. Client heartbeat and child-process polling
run on a separate low-frequency maintenance timer and do not request rendering.
Accepting a new Wayland client also sends one wake through that channel. The client can
already have protocol requests queued while being inserted into the Display, and the
Display source is not guaranteed to be dispatched again in the same calloop iteration.
Without this wake, a fully idle nested backend can defer the client's initial registry
and surface requests until the maintenance timer fires.

Upper layer-shell surfaces are already the frontmost prefix of Smithay's
`space_render_elements` scene. Mio preserves that ordering but inserts the cursor-wake
framebuffer effect after the Top/Overlay prefix and before ordinary Windows and lower
layers. Because render elements are composited back-to-front, the wake captures and
distorts the completed World scene, then Top/Overlay surfaces are composited once above
it. Do not redraw translucent upper layers or restore rectangular framebuffer regions:
redrawing darkens them, while rectangular restoration creates a stationary strip in
neighboring Window content.

While the cursor wake is active, the Top/Overlay elements keep the same geometry,
alpha, and final draw, but are wrapped to report no opaque region to Smithay's damage
tracker. Otherwise front-to-back opaque culling can omit the World pixels behind a bar;
displaced wake sampling near that edge then reads an undefined black area. Suppressing
only that occlusion hint keeps a valid World image beneath the upper layer without
distorting or redrawing the upper layer itself.

## Phase 15 framebuffer effects

Mio advanced to the 2026-03-09 revision because it is the first suitable pinned
revision providing `Element::is_framebuffer_effect` and
`RenderElement::capture_framebuffer`. `OutputDamageTracker` uses these hooks to recapture
the already-rendered background when content behind an effect changes. This avoids a
Mio-specific damage-tracking fork and keeps backdrop blur as an adapter render element.
The captured GLES texture is processed through renderer-created offscreen buffers. Each
level halves the previous dimensions, and custom texture shaders perform dual Kawase
downsample and upsample passes before the result is composited. These buffers live only
in the render element's `UserDataMap` cache and do not become compositor semantic state.

`Space::element_location` is the mapped content-geometry origin, whereas
`AsRenderElements::render_elements` expects the surface-tree render origin. When Mio
composites a blur Window's foreground separately, it therefore subtracts the Window's
relative `SpaceElement::geometry().loc` (and the Output origin), matching Smithay
`Space`'s internal `render_location` calculation. Passing `element_location` directly
makes decorated clients visibly jump only while their blur path is active.

For multiple simultaneous backdrop effects, Mio returns each Window's surface elements
followed by its blur element from `RenderWindow::render_elements`. Smithay consumes the
list front-to-back and draws back-to-front, placing the effect immediately behind its
own Window while retaining the surrounding Space and layer-shell z-order. This avoids
hiding blur Windows or maintaining a second renderer-side Window stack.

GLES texture-program overrides can wrap Smithay's existing Wayland surface elements
without replacing buffer import or surface-tree traversal. Mio uses this hook for
Window corner clipping and clears the override immediately after each delegated draw.
Its clip rectangle is derived from the presented `SpaceElement::geometry`, so camera
animation and render scaling remain the sole source of Window placement. The shader's
corner radius and the separate inner-border radius use the same presentation scale as
the surface; otherwise zooming out shrinks the Window beneath an effectively unscaled,
square effect boundary.

Soft Window shadows use Smithay's `GlesPixelProgram` rather than allocating a texture.
The fragment shader evaluates a rounded-box signed distance over bounds expanded by
three softness radii. Mio returns the shadow after the surface and backdrop elements;
because Smithay consumes that list front-to-back and draws it back-to-front, the shadow
lands directly behind its Window. Its stable per-Window element ID allows ordinary
damage tracking to account for the expanded bounds. Shadow radius, offset, and corner
radius follow Camera zoom because their geometry is built around an already zoomed
presentation rectangle; leaving them unscaled makes the shadow dominate distant
Windows.

Self-destroyed xdg-toplevels cannot keep rendering their live Smithay surface tree. Mio's
winit adapter therefore renders each committed Window into an offscreen `GlesTexture` and
retains the latest successful result. A surface commit marks only that Window's snapshot
dirty, avoiding a full copy on every output frame. `toplevel_destroyed` removes semantic
World state immediately and queues only the Window ID plus transition parameters; the
renderer pairs that notification with its cached texture and renders a short-lived
ClosingVisual through the same transition shader. These GLES objects and their animation
clock remain outside Mio Core.

The water transition uses one progress-only dissolve for both live Window elements and
ClosingVisual snapshots. The entire Window fades simultaneously as one water surface;
low-frequency refraction only softens that full-surface fade and never forms a directional
waterline. Opening is therefore the exact reverse of Closing rather than a direction-specific
shader branch.

The cursor wake is a framebuffer-effect render element placed at the protocol layer
boundary: below Top/Overlay layer-shell surfaces and above ordinary Windows and lower
layers. Launchers, bars, and notifications are therefore composed once and are never
sampled by the wake, without application exceptions or rectangular restoration. Recent
fast-motion samples are interpolated into a centerline and expanded along its normals
into left/right vertex pairs. A raw GLES triangle strip samples the captured output only
inside that single continuous ribbon. There is no fullscreen composite pass and
per-sample circles are not drawn. The normalized `cursor-wake-strength` setting is
converted to a small pixel displacement before division by the output size; treating
that value directly as pixels makes the configured default effectively invisible.

`CursorWakeElement::capture_framebuffer` performs the World-scene capture at the point
selected by Smithay's framebuffer-effect ordering; `draw` only consumes that capture.
Capturing independently from `draw` is required when the effect sits inside the scene:
copying the whole target during `draw` can include undefined or not-yet-rendered regions
and produces a black ribbon.

The ribbon itself uses a raw GLES triangle strip rather than Smithay's texture drawing
helper. Offscreen textures contain only mip level zero, while the GLES default
minification filter requires mipmaps. The raw path must therefore set linear min/mag
filters (and clamp-to-edge wrapping) explicitly before sampling; otherwise the texture
is incomplete and `texture2D` returns black across the ribbon even when capture worked.

The effects adapter keeps raw pointer input unchanged and retains the most recent fast
motion segments without moving or re-interpolating recorded points. Ordinary-speed
motion breaks the sampled path. Sample age controls width and opacity continuously: the
older tail is wider and fainter, while the head is narrower. This does not filter or
delay client pointer input.

The direct DRM backend expresses the same post-process as a full-Output framebuffer-effect
render element placed behind `Kind::Cursor` and Top/Overlay elements but ahead of the
World scene in the front-to-back list. This lets Smithay keep assigning the cursor to a
hardware plane while the primary-plane framebuffer is distorted. The element uses one
stable ID and a changing commit counter while active and retains its capture texture in
renderer-only cache state. The nested and direct paths use the same layer boundary and
neither path restores rectangular regions after the effect.

The revision also consolidates client DnD under `WaylandDndGrabHandler` and
`DndGrabHandler`, makes selection-state accessors mutable, renames the xdg-dialog
callback to `dialog_hint_changed`, and adds an Output serial-number field. Mio follows
the matching smallvil APIs for these compatibility changes.

## Phase 1 reference path

The matching upstream `smallvil` example is the primary starting point. At this
revision it uses these Smithay features with default features disabled:

- `backend_winit`
- `wayland_frontend`
- `desktop`

`backend_winit` pulls in the EGL/OpenGL renderer support used by the nested compositor.
This is a narrower initial feature set than Smithay's defaults and avoids enabling DRM,
libinput, X11 backend, XWayland, Vulkan, and other later-phase facilities.

The `smallvil` startup order is:

1. configure `tracing-subscriber`;
2. create `calloop::EventLoop`;
3. create `wayland_server::Display` and retain its handle;
4. construct compositor state and insert the Wayland display source;
5. initialize the winit backend/output/renderer;
6. run the calloop event loop.

## Phase 1 implementation findings

- A Wayland server must flush pending protocol events after each calloop dispatch cycle.
  Flushing only from a low-frequency maintenance timer delays configure, input release,
  and frame-related events by the timer period. Clients then resize in visible steps and
  may treat a normally released key as held long enough to repeat. Mio flushes from the
  `EventLoop::run` completion callback for both winit and direct DRM backends, matching
  anvil's dispatch/flush ordering; the maintenance timer is not a protocol-progress path.
- `Display` is registered as a level-triggered calloop `Generic` source. Access through
  `Generic::get_mut` is unsafe; the safety invariant is that calloop owns the Display
  source for the whole loop lifetime.
- The winit backend supplies absolute pointer motion, keyboard, button, axis, resize,
  redraw, focus, and close events needed by the nested development compositor.
- Mio's pointer edge-resize cursor uses Smithay's re-exported `CursorIcon` through an
  adapter-only override. The client `CursorImageStatus` is retained while this override
  is active, so leaving a handle restores the client's named or surface cursor.
During interactive resize, Core geometry still changes only when pointer motion crosses
a World Grid boundary. The adapter separately sends continuous pixel-size xdg configures
and maps an interactive presentation rectangle to the pointer. This keeps client content
reflowing without making frame-by-frame pixel geometry a second Core truth. Until the
client commits a requested size, the existing surface tree uses a uniform render scale;
independent x/y scaling made images and text visibly distort. Releasing the pointer ends
the preview and converges presentation on the final Grid-derived geometry.
Tiled followers are queried from Core's directional edge-adjacency derivation at grab
start and receive the same temporary pixel translation as the grabbed edge. The adapter retains only
their starting presentation rectangles for that grab; it does not duplicate adjacency
logic or persist a layout group.
- `Space<Window>` is temporary adapter-side geometry for Phase 1. It is not Mio's World
  source of truth and will be replaced by Core-driven mapping during Phase 3.
- `OutputDamageTracker` plus `space::render_output` renders xdg surfaces and the solid
  background. Frame callbacks are sent after submission and `Space::refresh` removes
  dead elements.
- A toplevel needs its initial xdg configure after its first surface commit. Popup
  configure and cleanup are managed through `PopupManager`.
- Smithay's `Window::render_elements` includes tracked popup surface trees before the
  toplevel tree. Mio's rounded-corner shader must therefore exclude every surface in
  those popup trees; applying the toplevel clip rectangle to all returned elements
  cuts menus off at the resized parent Window boundary. Popup placement and clipping
  remain protocol/render concerns and do not add Window state to Core.
- `Window::surface_under` returns its surface offset in the unscaled Window-local
  coordinate space. When Mio presents a Window through `RescaleRenderElement`, the
  offset from the pointer to that surface must be multiplied by the same presentation
  scale before deriving the global pointer-focus origin. Mixing unscaled local deltas
  with screen coordinates makes visible popup regions receive out-of-bounds local
  coordinates after an interactive resize.
- Camera animation can move or rescale a surface tree while the physical pointer stays
  still. After changing the presentation transform, Mio must issue pointer motion at
  the unchanged global logical position with a freshly computed surface origin. If it
  keeps Smithay's previous pointer focus origin, the next button event reaches the
  correct surface with stale pre-zoom local coordinates.
- A normal Window's visual gap must be scaled by the Camera zoom before deriving its
  presented rectangle, while its normal client configure keeps the unzoomed gap. This
  keeps the `RenderWindow` scale equal to the Camera scale, so Smithay's tracked popup
  and subsurface trees inherit the same transform as the toplevel. Scaling popup
  geometry separately would apply the Camera transform twice and break input origins.
- `KeyboardHandle::set_focus` is routed through the active Smithay keyboard grab.
  `PopupKeyboardGrab` ignores attempts to focus a surface outside its popup chain while
  that grab remains active. Mio verifies `current_focus()` before committing Core
  Focus, Camera following, stacking, and focus visuals. On rejection it restores both
  Core Focus and Smithay's pending focus to the actual protocol target.
- `Window::with_surfaces` and Mio's Window-ID lookup include tracked popup trees.
  Pointer Window-management gestures must still distinguish those popup surfaces:
  in particular, a primary click on a popup near its parent's edge must be forwarded
  to the client rather than starting the configured parent-Window resize action.
- On NixOS, winit's runtime loading requires the Wayland, xkbcommon, and EGL/OpenGL
  library paths configured by the repository's `shell.nix`.

Its implementation is split into compositor and xdg-shell handlers, input processing,
move/resize grabs, shared state, and the winit backend. Phase 1 should follow that
separation while using Mio names and only the required handlers.

## Reference order

For unfamiliar APIs, inspect the exact pinned revision in this order:

1. `smallvil` for a minimal nested compositor;
2. `anvil` for production backend and protocol patterns;
3. Smithay modules defining the API;
4. niri only where its behavior is relevant.

Do not copy Smithay into this repository or expose Smithay types from `mio-core`.

## Phase 3 integration findings

At the pinned revision, `Space::map_element` may be called again for an existing
`Window` to update its render location. The operation removes and reinserts the element,
so Mio explicitly raises the Core-focused window afterward to preserve stacking order.

`Space` clips mapped windows at output boundaries during `render_output`; no duplicate
surface or overview layout is required for a window crossing a Camera stop. Mio maps
the same Smithay `Window` clone at a Camera-relative position.

`XdgShellHandler::toplevel_destroyed` is the matching lifecycle hook for removing the
adapter mapping and Core Window. Initial and resized pixel sizes are written through
`ToplevelSurface::with_pending_state`, and the existing commit handler sends the initial
configure.

## Phase 4 protocol findings

At the pinned revision, fullscreen and maximize requests arrive through
`XdgShellHandler::{fullscreen,maximize}_request` and their corresponding `un*` hooks.
Mio resolves those requests into shared Core Actions, then writes the resulting
`xdg_toplevel::State` and pixel size through `with_pending_state`. This keeps client
requests and keyboard operations on one logical state path.

## Phase 6 rendering findings

`Space::render_elements_for_region` supplies one alpha to each mapped element. Mio maps
a small adapter `RenderWindow` that delegates Smithay's `SpaceElement` behavior to the
underlying `Window` and multiplies that alpha by the effective Core opacity. This keeps
per-Window opacity out of Mio Core's renderer-independent types while allowing two
Windows from the same app-id to render with different runtime overrides.

## Phase 8 zoom findings

Changing xdg-toplevel pending size during zoom makes clients reflow their contents and
is not a Camera zoom. Mio instead wraps every Wayland surface render element in
`RescaleRenderElement`, using the Window's render location as the scaling origin. The
client keeps its normal configured size while the complete surface tree, including its
contents, shrinks visually. Pointer hit coordinates are inversely scaled before being
forwarded to the surface.

## Phase 10 layer-shell findings

At the pinned revision, `WlrLayerShellState` owns the protocol global while desktop
`LayerSurface` values are mapped into the `LayerMap` associated with an `Output`.
`space::render_output` includes those maps automatically: overlay/top layers render
above the Space and bottom/background layers below it. Mio mirrors anvil by arranging
the map on commit before sending the mandatory initial configure. Pointer hit-testing
uses the same layer ordering explicitly because `Space::element_under` covers only
World Windows. Layer surfaces remain adapter-owned shell UI and never become
`mio-core` Windows.

Rendering a LayerMap does not itself complete client frame callbacks. Each mapped
LayerSurface, including its popups, must receive send_frame on the Output render loop
just like an ordinary Window. Without this, wofi displays its small bootstrap buffer
but stalls before repainting at the compositor's later configure size.

`LayerSurfaceCachedState::keyboard_interactivity` is committed before Mio's surface
commit hook runs. Top and Overlay surfaces requesting `Exclusive` must receive Seat
keyboard focus there. Mio also focuses an upper layer when it transitions into
`OnDemand` so persistent QuickShell launcher surfaces can accept input immediately,
but later commits do not steal focus back. Kaname's QuickShell 0.3.0 client sends
protocol value 2 (`OnDemand`) when its persistent PanelWindow becomes visible, even
though its QML requests `WlrKeyboardFocus.Exclusive` alongside `focusable: visible`.
When the focused layer is destroyed, Mio returns Seat focus to the still-selected Core
Window.

Smithay's compositor commit callback does not expose the pre-commit layer state after
the cache transition. Mio therefore records the last observed keyboard interactivity
per adapter-owned LayerSurface and compares it with the current request.
Mio also mirrors anvil's keyboard-event guard: a mapped upper layer whose current state
is `Exclusive` receives focus and the event before compositor keybindings are tested.

For xdg popups, the pinned smallvil/anvil implementations use
`find_popup_root_surface`, `get_popup_toplevel_coords`, and the positioner's
`get_unconstrained_geometry`. Mio applies the same calculation to both xdg-toplevel
and layer-shell roots. Because Overview scales completed surface trees, the available
Output rectangle is converted back through the root's render scale before updating
the popup's client-coordinate geometry.

`Space::outputs_for_element` can still be empty when `new_popup` arrives even though
the animated root Window is mapped and visibly intersects an Output. Popup
unconstraining therefore first uses Smithay's recorded association and falls back to
the Output with the largest intersection against the root's current Space geometry.
The fallback derives ownership from existing presentation geometry; it does not add a
second Window-to-Output state.

Tracking a popup does not implement `xdg_popup.grab`. Mio also follows anvil's Seat
grab path: validate that the root is a managed toplevel or layer surface, reject a
serial conflicting with an existing grab, and install `PopupKeyboardGrab` plus
`PopupPointerGrab`. This gives menus exclusive input and lets an outside click dismiss
the popup chain.

Entering a session lock must explicitly unset existing pointer, keyboard, and touch
grabs before lock-surface focus is installed. Changing keyboard focus alone does not
override Smithay grab dispatch, so retaining a popup, DnD, or constraint-related grab
could otherwise route input to a pre-lock client.

`XdgShellState::new` advertises every standard WM capability by default, including
minimize and a compositor-provided window menu. Mio uses `new_with_capabilities`
instead and advertises only fullscreen and maximize, whose requests have real Action
paths. Advertising the no-op default handlers would cause clients to expose controls
that cannot work.

`ToplevelState::bounds` is sent as `xdg_toplevel.configure_bounds` when supported by
the client. Mio derives it from the same Output render area used for normal,
maximized, or fullscreen presentation. It remains an adapter hint and does not become
another copy of Window geometry or Camera state.

The initial `xdg_toplevel.configure` must already contain Mio's selected client size.
Recording only an animation target delays `ToplevelState::size` until a later redraw;
some clients then remain at their self-selected small size until unrelated input
causes that redraw. Later resize animation may still emit incremental configures, but
initial mapping records and sends the final starting size synchronously.
The size sync also precedes `Space::raise_element(..., true)`, because activation may
itself emit the first configure; reversing those calls recreates the empty initial
configure even when the size is recorded immediately afterward.

Popup tracking, initial configure, and input-method dismissal return recoverable
errors when a client destroys a related resource mid-dispatch. Mio logs those errors
and continues serving other clients rather than silently hiding the failure or
escalating it into a compositor shutdown.

`ShellClient::send_ping` permits one pending ping and clears it before invoking
`client_pong`. Mio keeps adapter-only per-client deadlines, sends a ping every thirty
seconds, and logs once after a ten-second timeout. Mio deliberately does not call
`ShellClient::unresponsive`: a busy client such as a browser may deliver a late pong
and must not be killed merely for latency. Dead clients are removed independently; no
liveness state enters Mio Core or blocks other clients.

`ClientData::disconnected` distinguishes an ordinary closed connection from a
`ProtocolError`. Mio records normal exits at debug level and protocol failures at warn
level with the Wayland client identifier and backend error. This callback is
diagnostic only: it neither changes Core state nor turns one client failure into an
event-loop failure.

Pointer hit testing returns the deepest matching `wl_surface`, which may be a
subsurface or popup rather than the root xdg-toplevel. Mio uses Smithay Window's
`with_surfaces` traversal for the reverse Window lookup, so focus, Camera activation,
and pointer-constraint ownership work for the complete surface tree.

The pinned `ToplevelSurface::parent()` exposes the live `xdg_toplevel.set_parent`
relationship. Mio resolves it to an existing Core Window before removing a focused
dialog, then prefers that ID through the shared `FocusWindow` Action. If the parent is
absent or already destroyed, Core's ordinary removal fallback remains authoritative.
`XdgShellHandler::parent_changed` keeps an ordered derived return list current. Mio
prefers the direct parent, then its inherited ancestors, then the Window focused before
the new toplevel was activated. This also covers GTK dialogs that omit an xdg parent:
they return to the still-live previous dialog rather than Core's unrelated stable-ID
fallback.

Older GTK clients may use only `xdg_toplevel.set_parent` and never bind xdg-dialog.
Mio therefore treats a live toplevel parent, xdg-dialog modal state, or a fixed-size
toplevel as the same request for the existing floating Property. The pinned Smithay
stores client min/max constraints in `SurfaceCachedState`; Mio reads its current value
after each root surface commit and considers a Window fixed only when both axes have
equal, non-zero limits. Each update re-evaluates the OR of all hints, preventing one
unset event from tiling a dialog while another remains. Mio deliberately does not infer
dialogs from matching app IDs: GIMP, for example, uses the same app ID for startup,
ordinary, and dialog toplevels.

The renderer derives temporary per-axis scale from the geometry the client has actually
committed, rather than from the last configured size. A client may commit its initial
small buffer before processing a later configure; using the requested size as if it
were already committed renders the Window small until another state change (often
focus) causes a redraw. Fixed-size toplevels keep unit scale at normal Camera zoom and
use the Camera zoom in Overview, while remaining centered inside their World
presentation rectangle. This prevents splash screens from being enlarged to a full
tile without making them ignore Overview. Scale and centering are adapter presentation
state and do not modify Core World geometry.

`new_toplevel` occurs when the xdg role is created, before the client necessarily sends
its first buffer commit and final transient metadata. Mio sends its initial concrete
configure at role creation but defers Smithay Space mapping and activation until the
first root commit. Parent/fixed-size auto-floating therefore runs before the first
presentable frame instead of correcting a visible provisional tiled placement.
GTK may publish fixed min/max constraints only on the commit responding to activation.
For an unclassified new toplevel with a recorded return-focus candidate, Mio sets Core
focus and sends an activated configure after the first commit while withholding Space
mapping and Camera movement. The following commit either supplies the constraint or is
presented normally, so the delay is bounded rather than waiting indefinitely for a
particular hint.

The pinned Smithay revision implements staging `xdg-dialog-v1` through
`XdgDialogState` and `XdgDialogHandler::modal_changed`. Mio maps that hint onto its
existing `SetWindowProperty(Floating(true))` runtime Action and clears the same
Property layer on unset. Rendering and placement therefore use the ordinary floating
constraint in the same World rather than a dialog-specific layout.

## Phase 10 xdg-decoration findings

The pinned `XdgDecorationState` only owns protocol negotiation; it does not draw
decorations. Mio responds `ServerSide` to new, requested, and unset modes so clients
that support xdg-decoration omit their client-side title bars. Mio's intentionally
minimal server-side decoration consists of a subtle inner Window border and the focus light
rendered at the bottom edge: it adds no title bar or
window-control buttons. Close, resize, move, maximize, and fullscreen remain shared
Actions reached through configured keyboard, pointer, or IPC input rather than frame
widgets. A client that draws an application header bar as surface content cannot be
stripped by xdg-decoration.

The bottom water film and focus light are one render element of their `RenderWindow`, not an Output-level auxiliary
element. It consequently follows Space stacking and stays below top/overlay layer-shell
surfaces such as launchers. Adapter-only entry and exit progress may outlive a focus
change briefly, but it never changes Core focus or World geometry.

The Window border is likewise a `RenderWindow` element. It uses the presented Window rectangle
and corner radius, is composited above client content but below the focus light, and does not add
geometry or hit-test state to Mio Core.

## Phase 10 presentation-time findings

`PresentationState` advertises the clock id but does not complete callbacks itself.
For the normal single-Output path, Mio retains `render_output`'s
`RenderElementStates`, takes feedback only from surfaces associated with that Output,
and calls `presented` only after the winit backend submit succeeds. The same monotonic
clock supplies the advertised id and timestamps. Experimental virtual Outputs defer
feedback until their per-Output presentation attribution is reliable.

## Phase 10 cursor-shape findings

The pinned `CursorShapeManagerState` validates focus and serials, maps protocol shapes
to the shared `cursor_icon::CursorIcon`, then calls `SeatHandler::cursor_image` with a
`Named` status. Mio stores that adapter state and applies it to the nested winit host
Window. The delegate also has a `TabletSeatHandler` type bound even when Mio does not
advertise a tablet manager, so Mio supplies only the empty handler boundary.

Legacy `wl_pointer.set_cursor` surfaces carry a `CursorImageSurfaceData` hotspot.
Mio hides the host cursor and draws that surface tree in a final GLES pass above the
normal, virtual-Output, or lock frame. Frame callbacks follow the Output containing
the logical pointer. Cursor rendering occurs after screencopy readback, preserving the
current policy of omitting the cursor from captures. Entering session lock resets any
cursor surface left by an ordinary client before the locker can provide its own.

## Phase 10 selection findings

The standard `DataDeviceState` handles application clipboard and drag-and-drop. The
pinned Smithay revision provides `PrimarySelectionState` separately for select-and-
middle-click behavior, and `DataControlState` for clipboard tools and managers. Both
selection focuses must follow keyboard focus; Mio updates them together in the single
`SeatHandler::focus_changed` hook. Data-control is initialized with the same primary
selection state so clipboard tools can access both selections.

`ClientDndGrabHandler::started` supplies the optional drag-icon `wl_surface`; it is not
rendered by Smithay's data-device state automatically. Mio retains it only for the
grab lifetime and draws its surface tree in the same final pointer-overlay pass as a
legacy cursor surface. The icon origin follows the pointer directly, matching anvil;
the current cursor surface's hotspot belongs only to that cursor and must not offset an
independent drag icon. Drop, dead-surface cleanup, and session lock all clear the
adapter-owned reference.

Both nested and direct backends must include that surface tree in their final pointer
overlay and send it frame callbacks. Surface commits may carry `buffer_delta`; Mio
accumulates that value into the adapter-owned icon offset just as anvil does, without
turning the drag icon into Window or World state.

`LayerMap::non_exclusive_zone` returns the Output-local area left after layer-shell
exclusive zones are arranged. Mio uses that rectangle only for the adapter's
World-to-screen transform. Fullscreen bypasses it in favor of complete Output geometry;
Core Camera and Window geometry remain unchanged.

Mio hides Top layer surfaces for fullscreen by unmapping their desktop `LayerSurface`
from the Output `LayerMap`, while retaining the handles in adapter state. On fullscreen
exit it maps, arranges, and sends any pending configure to the same surfaces. This does
not disconnect or recreate the layer-shell clients; Overlay remains mapped.

## Phase 10 session-lock findings

The pinned `SessionLockManagerState` deliberately separates accepting a request from
`SessionLocker::lock()`. Mio keeps the confirmation pending, renders an opaque black
frame instead of every ordinary Window and LayerSurface, submits that frame, and only
then sends `locked`. Lock surfaces are adapter-owned per Output and never enter Core.
While locked, keyboard shortcuts and ordinary hit-testing are bypassed. A vanished
locker remains fail-secure (black); only the protocol's explicit unlock restores the
selected Core Window and its Seat focus.

The direct DRM adapter follows the same ordering by switching its render-element list
to only the matching `LockSurface` trees and an opaque-black clear. It excludes the
ordinary Space, layer-shell surfaces, and closing snapshots before queueing the KMS
frame. The current cursor remains above the lock UI without exposing desktop
content. Session-lock request, surface creation, and explicit unlock each wake the
backend; lock-surface frame callbacks are sent instead of callbacks for hidden desktop
content while the lock remains active.

`ext-session-lock-v1` configures lock surfaces in logical Output coordinates. The
configured size must therefore divide the physical mode size by the Output scale, and
the completed surface tree must be rendered with that same Output scale. Configuring
the physical mode size and rendering at scale `1.0` only happens to work at scale
`1.0`; fractional scale otherwise produces an oversized or incomplete lock UI.

## Phase 10 idle-inhibit findings

`IdleInhibitManagerState` owns the `zwp_idle_inhibit_manager_v1` global but, unlike
several other Smithay protocol states, its handler has no state getter. Mio retains the
manager for its lifetime and counts the `inhibit`/`uninhibit` callbacks by surface.
Smithay notes that explicit uninhibit is the callback boundary; a future idle policy
must additionally ignore invisible surfaces before suppressing an idle action. Mio
already removes inhibitors whose Wayland surfaces die without an explicit uninhibit
during its normal frame cleanup.

## Phase 10 viewporter findings

At the pinned revision `ViewporterState` is self-contained: constructing its global and
using `delegate_viewporter!` installs viewport state into normal Wayland surface state,
which Smithay render elements already consume. No Mio handler callback or Core mapping
is required. In particular, a client's buffer source/destination is distinct from
Mio's Camera zoom and Window `GridRect`.

## Phase 10 fractional-scale findings

`FractionalScaleManagerState` calls the compositor when a surface creates its scale
object. Mio selects the current Output scale and writes it through
`with_fractional_scale`. Client buffer viewport, fractional Output scale, Mio Camera
zoom, and Window `GridRect` are four distinct concerns.

Pointer state remains in logical Output coordinates. Cursor surfaces and drag icons
must convert their logical origin with `Output::current_scale().fractional_scale()` and
pass the same `Scale` to `render_elements_from_surface_tree`. The direct backend's
software cursor follows the same rule for its logical hotspot. Rendering these overlays
with a hard-coded scale of `1.0` makes the visible cursor stop at the logical bottom edge
and separates its hotspot from the logical click position on fractionally scaled Outputs.
Cursor-wake history also remains logical. Its render element uses the logical Output size,
then converts the centerline, configured width, and cursor-sized head to physical coordinates
when building the shader mesh. Treating those history values as physical pixels offsets and
narrows the wake relative to the scaled cursor.

## Phase 10 input-method findings

At the pinned revision, `TextInputManagerState` and `InputMethodManagerState` share
per-Seat `TextInputHandle` and `InputMethodHandle` values. Smithay forwards text-input
enable/state/commit to the input method and forwards preedit, committed text, deletion,
and keyboard grabs back to the focused client. Both globals therefore need to be
enabled together.

Fcitx5 also expects `zwp_virtual_keyboard_manager_v1` for its native Wayland input
path; its self-diagnostic reports a native protocol count of zero when Mio exposes
only text-input and input-method. The pinned anvil initializes all three globals
together. Mio follows that set for the nested backend. All clients can currently see
the privileged input-method and virtual-keyboard globals; a production backend must
restrict them to trusted clients.

Input-method candidate surfaces are `PopupKind::InputMethod` values. Tracking them in
the existing `PopupManager` makes Smithay's normal Window and layer-shell render paths
include them without creating a Mio Core Window. `InputMethodHandler::parent_geometry`
supplies the focused parent surface geometry. Pointer-driven focus changes remain
allowed while the special input-method keyboard grab is active, matching anvil.

## Phase 10 screencopy findings

The pinned Smithay revision has no legacy wlr-screencopy server state or anvil example,
even though its re-exported `wayland-protocols-wlr` contains the protocol bindings.
Mio's initial legacy implementation therefore owns that protocol dispatch in its
adapter instead of adding anything to Smithay or Mio Core.

The same revision provides Smithay's standard `ext-image-copy-capture-v1` server state,
based on COSMIC Comp, together with output capture sources and protocol delegates. Mio
uses those types as its primary portal screencast path and supplies only the
compositor-specific constraints, session lifetime, final-scene SHM readback, and
presentation timestamp. xdg-desktop-portal-wlr 0.8.4 selects this standard path when
both manager globals are present and falls back to legacy wlr-screencopy otherwise.

For the nested GLES backend, capture requests are queued during Wayland dispatch and
fulfilled after `render_output` and before backend submission. `ExportMem` reads the
final framebuffer into ARGB8888 memory, and Smithay's SHM access helpers validate and
write the client buffer. Region coordinates are converted from output top-left to the
GLES framebuffer's bottom-left origin. The winit backend already renders with
`Transform::Flipped180`; advertising the GLES mapping's generic `flipped()` value as
the screencopy `y_invert` flag makes grim invert the completed Output a second time.
The nested capture therefore exports its observed final row order with no `y_invert`
flag. The first implementation reports full region damage and has no separately
rendered cursor to composite.

The screencopy `ready` event uses the absolute `CLOCK_MONOTONIC` timestamp from Mio's
Smithay `Clock<Monotonic>`. A duration measured from compositor startup is valid for
surface frame callbacks but not for this protocol event. Passing that relative value
lets one-shot screenshot clients succeed while PipeWire consumers treat subsequent
frames as stale and can leave a screencast frozen on its first image.

After a requested frame is copied or fails, the compositor must also send
`wl_buffer.release` for the supplied SHM buffer. The screencopy `ready` event only
completes the frame object; it does not release that buffer. Portal screencasts use a
small PipeWire buffer pool, so omitting `release` exhausts the initial buffers and
freezes the stream even though one-shot screenshot clients still work.

`GlesRenderer::map_texture` makes the EGL context current without the winit window
surface. `WinitGraphicsBackend::bind` only constructs a GLES target and does not make
the EGL surface current by itself. After a readback Mio therefore binds the target and
starts/finishes an empty render frame before `submit`; otherwise the following swap
fails with `EGL_BAD_SURFACE` and context loss.

Both backends advertise the standard and legacy capture globals. Sandboxed
applications use xdg-desktop-portal-wlr's selection flow; an unsandboxed process that
can connect directly to Mio's ordinary Wayland socket is treated as part of the same
desktop-session trust boundary and can access the protocols directly. This is not
per-client authorization, so deployments must protect access to the session socket.
Locked sessions reject capture requests.

When a direct-backend request is pending, Mio renders the already assembled scene,
excluding the pointer overlay, into a temporary GLES texture and feeds that
framebuffer to the shared SHM readback implementation. The normal DRM scanout remains
unchanged.

## Phase 10 linux-dmabuf findings

The pinned winit anvil initializes DMA-BUF only after creating its GLES renderer.
Mio follows that ordering: formats come from `GlesRenderer::dmabuf_formats`, and an
EGL render node supplies the main device for v4 default feedback. If the render node
or feedback cannot be obtained, Smithay can expose the same renderer formats through
the v3 global instead.

DMA-BUF create requests arrive during Wayland dispatch, when Mio's renderer is owned
by the winit callback rather than `MioState`. The handler therefore queues each
`Dmabuf` and `ImportNotifier`; the next redraw imports it through the actual renderer
and reports success only after that succeeds. This keeps renderer ownership out of
Core and avoids accepting an unusable client buffer. Smithay's `backend_drm` and
`use_system_lib` features are needed for render-node discovery and EGL's Wayland
display binding at this revision.

## Phase 10 single-pixel-buffer findings

At the pinned revision `SinglePixelBufferState` owns the complete protocol global and
buffer dispatch. `WaylandSurfaceRenderElement` recognizes its `wl_buffer` user data
and creates the solid-color texture through the ordinary renderer path. Mio therefore
only initializes and delegates the protocol; no Core Window state or custom render
element is required.

## Phase 10 alpha-modifier findings

`AlphaModifierState` stores the committed multiplier in each Wayland surface's cached
state. At the pinned revision `WaylandSurfaceRenderElement` reads it and multiplies
the render element alpha automatically. Mio only initializes and delegates the
protocol. This remains distinct from Mio's effective Window Property opacity; the
existing outer render alpha composes with the client's per-surface multiplier.

## Phase 10 xdg-toplevel-icon findings

The pinned `XdgToplevelIconManager` validates that pixel icons use square SHM buffers,
freezes icon builders when assigned, and stores the chosen name/buffers in
`ToplevelIconCachedState` on the xdg toplevel's `wl_surface`. Mio advertises common
32/64/128/256 logical-pixel preferences and otherwise relies on this implementation.
The icon remains Wayland adapter metadata; it is not duplicated in Mio Core. A future
shell IPC extension may serialize a themed name or selected pixel representation.

## Phase 10 content-type findings

The pinned `ContentTypeState` validates one role object per `wl_surface` and commits
the client's None/Photo/Video/Game value through `ContentTypeSurfaceCachedState`.
Initialization and delegation are sufficient to retain the metadata. The current
nested Output has no scanout, color, or refresh policy that should consume it, so Mio
does not invent behavior or copy the value into Core.

## Phase 10 xdg-foreign findings

The pinned `XdgForeignState` implements v2 exporter/importer globals, opaque handle
generation, relationship invalidation, and parent assignment through the existing
`XdgShellHandler`. Mio only supplies the state accessor and delegation. These transient
xdg-shell relationships do not imply World Window ownership, lifetime, or a layout
container.

## Phase 10 XDG activation findings

At the pinned revision, XdgActivationState owns the protocol global and stored token
data, while the compositor supplies token acceptance and activation policy through
XdgActivationHandler. Anvil validates that a client token names the compositor Seat
and carries a serial no older than the keyboard's last enter serial, then applies a
short age limit when activation is requested.

Mio follows that validation, uses a ten-second limit, consumes the token on the first
request, and ignores unmanaged target surfaces. Accepted requests resolve the target
to its stable Core WindowId and enter the existing Focus plus Camera-center path;
XDG-specific state does not enter mio-core.

## Phase 10 touch findings

The winit backend emits the standard Smithay touch event variants, but adding
`TouchFocus` to `SeatHandler` alone does not advertise or deliver touch. Mio adds one
TouchHandle to its Seat and forwards down, motion, up, frame, and cancel events. Down
and motion positions use the same Output transform and surface hit-test as absolute
pointer motion; a down also enters Mio's existing Window activation path.

## Phase 10 keyboard-shortcuts-inhibit findings

The pinned manager stores inhibitors in Seat user data and exposes lookup by the exact
`WlSurface`. Mio retains protocol handles so `SeatHandler::focus_changed` can activate
only the inhibitor belonging to the current keyboard-focus surface and inactivate the
others. Configured bindings are bypassed only for that active inhibitor. Session lock
forwarding and exclusive upper-layer focus are evaluated first, so a normal client
cannot weaken either policy.

## Phase 10 relative-pointer and pointer-constraints findings

The pinned Smithay revision exposes relative motion through
`PointerHandle::relative_motion`; creating the relative-pointer global alone does not
generate events. Mio derives a delta from consecutive absolute winit positions and
sends it before ordinary pointer motion. `with_pointer_constraint` stores one
constraint per surface and pointer. Mio activates it only while that surface has
pointer focus and its committed region contains the pointer. Active locks retain the
logical location while still emitting relative motion; active confinement rejects a
candidate outside the same surface or region. Smithay automatically deactivates a
constraint when pointer focus leaves its surface.

The nested winit backend reports host-window absolute cursor positions and has no raw
unaccelerated device delta. Mio therefore reports the derived delta for both protocol
fields, and cannot continue producing locked motion once the host cursor reaches its
window edge. A future libinput backend can remove that limitation without changing
Core or protocol state.

## Phase 11 xwayland-satellite findings

Xwayland-satellite is itself a Wayland client and requires only the ordinary core,
xdg-shell, and viewporter protocols for its base path. Mio therefore does not enable
Smithay's built-in `xwayland` feature or introduce an X11-specific Core Window type.
Satellite-created X11 compatibility windows arrive through Mio's existing xdg-shell
adapter and follow the same World placement, focus, resize, and Camera behavior.

The satellite CLI accepts an X display such as `:100` as its first argument. Mio's
initial explicit startup integration passes its own `WAYLAND_DISPLAY`, publishes the
same X `DISPLAY` only to subsequently spawned applications, polls child liveness without
terminating native Wayland on failure, and reaps the child at shutdown. Collision-free
listen-fd activation and restart policy remain deferred.
Without a live satellite, Mio removes the host compositor's inherited `DISPLAY` from
its startup application. This prevents an X-preferring client from silently opening
on the outer niri session while preserving the normal Wayland socket inheritance.
`WAYLAND_DISPLAY`, `MIO_SOCKET`, and the optional `DISPLAY` are assigned directly on
that child command; Mio does not rewrite its own process-wide environment after winit
and EGL initialization. Descendants of the startup application inherit the same Mio
endpoints normally.
When polling observes satellite exit, Mio clears both the child handle and the
associated X display endpoint. Shutdown does the same after reaping the process, so
adapter state never claims that a dead compatibility server is available.

Mio enables calloop's Linux signal source at the same 0.14 dependency used by the
pinned Smithay revision. Registering SIGINT and SIGTERM before winit initialization
routes terminal and service-manager shutdown through normal event-loop cleanup rather
than an asynchronous handler or an additional lifecycle subsystem.

## Phase 14 nested multi-Output findings

At the pinned Smithay revision, desktop space render_output derives one set of
SpaceElement locations for one Output. Mapping the same Window into multiple Space
positions would duplicate adapter state and cannot represent independent Camera
transforms cleanly.

The nested development path instead asks each RenderWindow for its ordinary surface
render elements once per Camera, applies a derived RescaleRenderElement, and wraps it
in CropRenderElement for that virtual Output's host-window region. Rendering these
views directly into the one winit framebuffer preserves one Core Window and one
Wayland surface tree while allowing simultaneous Camera views. The ordinary
render_output path remains unchanged when only one Camera exists. The first virtual
split was implemented before distinct wl_output globals; the following slice connects
those globals while retaining the same derived render path.

Each derived view needs its own adapter-side AnimatedRect. Using the active Output's
target rectangle directly makes Camera motion jump even though the ordinary Space path
still interpolates; sharing one rectangle also cannot represent two Camera targets.
These per-view rectangles are render state only and never feed positions back into Core.

The next nested slice creates one Smithay Output per virtual region, assigns each a
region-sized Mode and host-relative logical location, maps all of them into the shared
Space, and retains an OutputId-to-Output adapter table. Layer surfaces can therefore
select and arrange against the correct Output. Because all virtual Outputs still share
one winit framebuffer, screencopy converts the requested Output-local region through
that Output's host framebuffer origin before GLES readback. Physical backend discovery
and hotplug are separate from this nested representation.

Fullscreen and maximized presentation keep the Core Camera unchanged, but their
adapter-side rectangles must be the selected Output area directly. Passing the
viewport-sized World rectangle through the Camera transform both reapplies overview
zoom and offsets it when a continuously panned Camera position differs from the
integer viewport origin. Normal Windows alone use the Camera transform; presented
Windows use the exact full or non-exclusive Output area for ordinary and virtual
render rectangles.

## Phase 14 direct DRM/KMS bootstrap findings

The pinned Smithay revision's `anvil` backend keeps four objects distinct: a
`LibSeatSession`, a `DrmDevice` event source, a `DrmOutputManager`, and each initialized
`DrmOutput`. Mio follows that ownership in its adapter. Session pause suspends libinput
and pauses the manager; activation resumes libinput, reactivates the manager, and then
requests a new frame. None of these backend objects enter Mio Core.

The initial direct path uses `primary_gpu`, opens the seat-owned DRM node, creates a
GBM allocator and framebuffer exporter, scans one connected connector/CRTC pair, and
uses its preferred mode. A Smithay `Output` is mapped to the existing Core Camera, so
physical startup does not introduce a workspace or a second Window model.

DMA-BUF feedback must be created from the GLES renderer's supported formats and the
DRM node device id, after renderer initialization. EGL is also bound to Mio's Wayland
display so EGL-buffer clients can use the same import path. Pending imports continue
through Mio's existing protocol-side queue and are accepted only when the direct
renderer imports them successfully.

The direct renderer composes ordinary Space and layer-shell elements through
`DrmOutput::render_frame`, queues a page flip, and renders again after VBlank or an
explicit input/client wake. Client cursor surfaces are composed as cursor elements.
Named cursors are resolved lazily from the configured XCursor theme; all frames at the
nearest nominal size and their per-frame delays are retained, and animated cursors keep
the paced repaint loop armed. `Kind::Cursor`, the GBM device supplied to
`DrmOutputManager`, and `FrameFlags::DEFAULT` let Smithay promote a fitting cursor
element to a DRM cursor plane. If the device has no cursor plane or the element exceeds
its size, Smithay composites that same element into the primary plane instead. Mio logs
assignment transitions at debug level. A built-in pointer is used when the default
theme image is unavailable. Multiple connectors/GPUs remain a later direct-backend
slice.
Window-local effects use the shared `RenderWindow` elements. Closing transitions keep
the last per-Window offscreen snapshot in the direct adapter and place its transition
element above the live Space until the configured duration expires; this preserves the
same Core removal semantics as the nested backend.

Receiving `DrmEvent::VBlank` is not by itself enough to finish a page flip. The backend
must call `DrmOutput::frame_submitted()` before attempting the next render/queue cycle.
Without it, the initial modeset frame can appear while every later client and cursor
update remains stuck behind the swapchain's submitted frame. Mio then clears its own
pending marker and schedules the next render only after Smithay has advanced that
output state.

Frame callbacks are likewise output-paced. They are normally sent after a queued frame
completes at VBlank, using the Output mode's refresh interval. An empty render queues
no page flip and therefore produces no later VBlank; following the pinned anvil
backend's repaint scheduling, Mio retries that path at approximately the output cadence
and sends callbacks from the paced retry. Sending callbacks immediately after every
empty render instead creates an unbounded client commit/callback loop: layer-shell and
terminal clients continuously wake the compositor, input release processing is delayed,
and keyboard repeat can appear to stick. The paced retry also keeps compositor-side
animations moving when damage tracking temporarily reports an empty frame.

The focus-light reveal is an adapter-side animation and must participate in the same
repaint-continuation decision even though it does not change Mio Core geometry.

The direct adapter retains its `DrmScanner` after startup. A matching disconnect drops
the `DrmOutput` (which removes that CRTC's compositor from `DrmOutputManager`), unmaps
the Smithay Output, and removes its Wayland global while leaving Mio's Core Output and
Camera identity intact. A later connection selects the preferred mode, creates a fresh
`DrmOutput`, remaps and re-advertises the same Smithay Output, updates layout size, and
requests repaint. A `Changed` event recreates the scanout the same way so a refreshed
mode list is applied. The same connection path is used when startup discovers no
connected connector: Mio keeps the DRM scanner and event loop alive without advertising
a placeholder `wl_output`, then creates and advertises the first real Output on hotplug.
CLI and configured startup commands are retained until that Output is mapped; clients
such as terminals are therefore not launched into a Wayland display with no Outputs.
Additional connectors remain ignored by the explicitly
single-Output adapter; multi-Output policy stays a separate Phase 14 slice.

Unlike winit's absolute host-window pointer events, libinput normally emits relative
pointer motion. The direct adapter clamps the accumulated position to the Output and
passes both accelerated and unaccelerated deltas through the shared pointer path.
VT keysyms are backend control rather than Mio Actions; the shared keyboard filter
returns a `ChangeVt` request which only the libseat adapter executes.
