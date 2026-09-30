# AGENTS.md

# Mio / 澪

Mio is a Rust + Smithay Wayland compositor / tiling window manager.

Before changing architecture or behavior, read:

- `docs/requirements.md`
- `docs/spec.md`

For Smithay-specific implementation notes, read:

- `docs/smithay-notes.md`

When this file conflicts with assumptions, examples, or guessed behavior, follow this file and the current project documentation.

---

## Core rule

Mio prefers **a small number of simple, composable principles** over many special-case features.

Before adding a new concept, mode, subsystem, or config option, ask:

> Can this be expressed by composing existing Mio primitives?

Do not invent new Mio behavior when requirements are unclear. Preserve the simpler existing behavior or consult the project documentation.

---

## Core concepts

The primary Mio concepts are:

- World
- Grid
- Window
- Camera
- Focus
- Selection
- Property
- Action

High-level behavior should be derived from these concepts whenever possible.

Examples:

- Overview = Camera zoom / transform.
- Moving between areas = Camera movement.
- Floating = relaxed Grid constraint.
- Runtime appearance changes = Window Property overrides.
- Keyboard, mouse, IPC, and future Yaldra integration = shared Actions.

---

## No traditional workspaces

Mio has one continuous 2D World.

Windows live at World coordinates.

Camera stops may resemble workspaces, but they are not containers.

Do not introduce a primary model such as:

```rust
struct Workspace {
    windows: Vec<Window>,
}
```

Windows must be able to cross Camera viewport boundaries without changing identity or ownership.

Camera visibility is not Window lifetime.

---

## Keep `mio-core` independent

`mio-core` must not depend on Smithay.

Do not expose Smithay / Wayland / DRM / renderer-specific types inside Mio Core.

Preferred boundary:

```text
Smithay / Wayland
       ↓
Adapter
       ↓
Mio Core
```

Mio Core should remain testable as ordinary Rust logic.

---

## One source of truth

Each semantic state must have one clear source of truth.

Avoid duplicate mutable copies of:

- Window geometry
- Camera position
- Focus state
- Floating state
- Window properties

Logical state and render/interpolation state may differ, but they must be clearly separated.

Example:

```text
logical_position
target_position
render_position
```

Animation must not mutate core World geometry frame-by-frame.

---

## Actions

User-facing operations should flow through the shared Action model.

Preferred structure:

```text
Keyboard / Mouse / IPC / Yaldra
             ↓
           Action
             ↓
          Mio Core
```

Examples:

- `Focus`
- `MoveWindow`
- `ResizeWindow`
- `CameraStep`
- `CameraTo`
- `CameraZoom`
- `ToggleFloating`
- `CloseWindow`
- `SetWindowProperty`
- `ClearWindowProperty`

Do not independently reimplement equivalent behavior for different input methods.

---

## Window properties

Use one common Property system for Window behavior and appearance.

Property precedence:

```text
Default
↓
Matched Config Rules
↓
Runtime Override
```

Runtime overrides are per-window unless explicitly specified otherwise.

Changing the focused Window must not silently modify every Window with the same app-id.

Do not rewrite the user's handwritten KDL file for temporary runtime changes.

---

## Tiling / floating

Mio is primarily tiled.

Tiled Windows are constrained to the World Grid.

Floating Windows remain in the same World.

Do not create a separate floating workspace or unrelated coordinate model.

Prefer:

```text
tiled    = grid constraint enabled
floating = grid constraint relaxed
```

---

## Camera

The display is a Camera looking at the World.

Camera stop boundaries are navigation boundaries only, not Window layout boundaries.

Windows may cross them normally.

Overview must preserve the same World and Window geometry.

Do not build a duplicated Overview layout or Window representation.

Prefer Camera position / zoom transforms.

---

## Configuration

KDL is for declarative configuration.

Future Yaldra integration is for programmable behavior.

Do not require Yaldra for normal Mio operation.

Keep config options high-level. Do not expose every internal implementation parameter by default.

Do not create separate configuration systems for appearance rules, floating rules, blur rules, etc. Prefer shared Window Rules + Properties.

---

## Smithay

Smithay is an upstream dependency, not Mio source code.

Do not modify or vendor upstream Smithay unless explicitly instructed.

Reference repositories such as `../smithay` and `../niri` are read-only unless explicitly stated otherwise.

Before using unfamiliar Smithay APIs:

1. Check the revision pinned by Mio.
2. Inspect the matching Smithay source.
3. Check `smallvil`.
4. Check `anvil`.
5. Check niri when relevant.

Do not guess APIs from memory or unrelated versions.

Record important Smithay findings in `docs/smithay-notes.md`.

---

## X11

Prefer `xwayland-satellite` initially for X11 compatibility unless requirements change.

Do not let X11-specific behavior pollute Mio Core.

From Mio Core's perspective, compatibility Windows should behave like ordinary Windows whenever possible.

---

## External shell

Mio must not require Shirube or Kaname.

Do not move bar, launcher, notification, or system-status UI into Mio Core.

Expose compositor state and Actions through IPC instead.

---

## Visual effects

Visual effects must be optional and must not be required for Window Management correctness.

Blur, ripple, shadow, pseudo-3D effects, and similar features belong to rendering/effects layers.

Mio's World remains 2D.

Do not introduce 3D Window placement, Camera rotation, depth layout, or occlusion as Core concepts.

---

## Scope discipline

Follow the implementation phase defined in `docs/requirements.md`.

Do not start major later-phase features unless explicitly requested.

Examples:

- Do not implement blur before core World/Camera behavior is stable.
- Do not implement Yaldra before Action/Property APIs are stable.
- Do not add elaborate Overview UI; Overview is Camera behavior.
- Do not build shell features into the compositor core.

Avoid unrelated refactors while implementing a focused task.

Prefer small, reviewable changes.

---

## Dependencies

Do not add major dependencies without justification.

Before adding a crate:

1. Check whether std or existing dependencies are sufficient.
2. Explain why it is needed.
3. Prefer small, maintained crates.
4. Avoid dependencies for trivial helpers.

---

## Tests

Pure Mio Core logic should have unit tests.

Test relevant behavior for:

- negative World coordinates
- Window movement
- Window resize
- Camera movement
- Camera boundary crossing
- directional focus
- Window removal
- Property precedence
- Runtime override clearing

Bug fixes in Mio Core should normally include a regression test.

---

## Error handling

Do not silently ignore important errors.

Avoid unnecessary `unwrap()` / `expect()` in long-lived compositor paths.

Configuration errors should be actionable.

A client failure should not crash the compositor unless unavoidable.

---

## Documentation

Update project documentation when changing:

- Core concepts
- Camera semantics
- Action model
- Property precedence
- configuration structure
- IPC semantics
- Smithay integration strategy

Do not let implementation and architecture documentation drift apart.

---

## Definition of done

Before reporting a task complete:

1. Run formatting.
2. Run relevant tests.
3. Build affected workspace crates.
4. Check for obvious regressions.
5. Verify this change does not violate Mio architecture.
6. Confirm it stays within the current implementation phase.
7. Summarize what changed.
8. State known limitations or deferred work.

Compilation alone does not mean the task is complete.

---

## Priority order

When tradeoffs exist, prefer:

1. Conceptual simplicity
2. Internal consistency
3. Correctness
4. Daily usability
5. Maintainability
6. Extensibility
7. Visual novelty

Mio should become more capable without requiring users or maintainers to learn an ever-growing number of concepts.

---

## Core motto

> The world is one. Windows live in it. The display is only a camera looking into that world.

> Increase capability without increasing the number of concepts users must understand.