# Mio project map
- Rust + Smithay Wayland compositor/tiling WM. Motto: one continuous 2D World; Windows live in it; each display is only a Camera.
- Read `docs/requirements.md` and `docs/spec.md` before behavior/architecture work; read `docs/smithay-notes.md` before unfamiliar Smithay work. `AGENTS.md` is binding.
- Workspace crates: pure logical model `crates/mio-core` (see `mem:mio-core/core`); Smithay adapter/runtime `crates/mio-compositor` (see `mem:mio-compositor/core`).
- Public/user docs are under `docs/`; distributed KDL is `config/mio.kdl`; packaging is `flake.nix`, `nix/`, `install.sh`.
- Scope is stabilization/1.0 preparation. Phase 18 public-surface documentation audit is complete, but physical multi-monitor stability, IME stability, and sustained daily-use acceptance remain blockers; consult `docs/release-readiness.md`, not the older “Current phase” sentence in `docs/architecture.md`.
- No traditional workspaces, duplicate overview layout, floating workspace, or 3D World. External shells such as Shirube/Kaname are optional.
- Development environment and dependencies: `mem:tech_stack`; commands: `mem:suggested_commands`; conventions: `mem:conventions`; completion checks: `mem:task_completion`.