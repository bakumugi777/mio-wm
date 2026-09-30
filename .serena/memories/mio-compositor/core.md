# mio-compositor
- Smithay adapter/runtime: Wayland protocols, input, KDL parsing, rendering/effects, animation, IPC, winit nested backend, udev DRM backend, screencopy, and xwayland-satellite integration.
- `state.rs` associates adapter-owned Wayland objects and derived render state with Core Windows. Do not duplicate semantic geometry/focus/properties.
- Keyboard, pointer, IPC, and future programmable integration must construct/compose shared Core Actions; do not reimplement equivalent WM semantics per input path.
- `config.rs` owns declarative KDL loading/validation/reload. Replacement config is validated before commit; errors remain actionable adapter state. Appearance/effects are presentation state.
- `input.rs` translates gestures and keybinds. Temporary drag/resize previews are adapter render state; release commits via Core Actions.
- `winit.rs` is nested rendering; `udev.rs` is direct DRM/KMS. Backend-specific resources/effects never enter Core.
- Smithay is pinned upstream and read-only. Before unfamiliar APIs inspect the pinned revision, then smallvil/anvil, then niri where relevant; record durable findings in `docs/smithay-notes.md`.
- Effects are optional and cannot affect Window Management correctness or input coordinates.
- IPC reads existing state and maps mutations to Actions; it must not own a duplicate World cache.