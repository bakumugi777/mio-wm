# Definition of done
For affected Rust work, run in order:
1. `cargo fmt --all -- --check`
2. `cargo clippy --workspace --all-targets -- -D warnings`
3. `cargo test --workspace`
4. `cargo build --workspace`

Also:
- Run focused regression tests while developing (normally `cargo test -p mio-core` for pure logic).
- Validate relevant KDL with `cargo run -p mio-compositor -- --config config/mio.kdl --check-config` when configuration changes.
- Run `nix flake check` when package/module/installer behavior changes.
- Check obvious regressions, architecture invariants, and current phase scope; compilation alone is insufficient.
- Update required docs for Core concepts, Camera, Actions, Properties, configuration, IPC, or Smithay integration.
- Report changed behavior, verification performed, and known limitations/deferred work.