# mio-core
- Smithay-independent source of truth for World, Grid, Window, Camera, Focus, Output cameras, Presentation, Property precedence, and Action application.
- Core must never expose Smithay/Wayland/DRM/renderer/backend types.
- Main orchestration is `World` in `src/world.rs`; user operations enter through `Action` in `src/action.rs`.
- Window geometry is World state. Visibility is derived from Camera intersection and never owns Window lifetime. World coordinates may be negative.
- One Camera per Output, one active Output for Camera Actions/placement; Windows are not owned by Outputs.
- Tiled and floating share one World/coordinate model; floating is relaxed Grid constraint. Resize adjacency/follower chains are derived per Action and applied atomically, not stored as layout groups.
- Property precedence is Default < matched Config Rule < per-window Runtime Override. Runtime changes do not rewrite KDL.
- Logical geometry and adapter render/interpolation state must remain separate. Core geometry is never mutated frame-by-frame for animation.
- Pure behavior changes normally require unit/regression tests in the relevant core module, including negative coordinates and atomic failure behavior.