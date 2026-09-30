# Tech stack
- Rust 2021 workspace, MSRV 1.85, resolver 2; crates `mio-core` and `mio-compositor`.
- Smithay pinned to git revision `f217f62bbe3f5c414997b91d1fe9caeb5e8662d3`; do not guess APIs from other revisions.
- Core dependencies are deliberately minimal; compositor uses Smithay, calloop 0.14, KDL 4.7.1, tracing, xkbcommon, xcursor.
- Build/package management: Cargo plus Nix flake/dev shell. NixOS module and package live under `nix/`; non-Nix installer is `install.sh`.
- Configuration is declarative KDL. Runtime integration uses Unix-socket IPC and `mioctl`.
- Linux/Wayland only; winit nested backend for safe development, udev/libseat/DRM direct backend for real sessions; X11 compatibility initially via xwayland-satellite.
- Dev shell includes Cargo, rustc/rustfmt/clippy/rust-analyzer and required Wayland/DRM/input/GL libraries; NixOS runtime library lookup depends on the shell’s `LD_LIBRARY_PATH`.