# convert-to-workspace

## Summary
Convert single-crate project to Cargo workspace with 3 crates.

## Design
Split into flat crates/* layout:
- crates/flint-gate-core/ — lib: config, auth, stream, proxy, cache, db, admin
- crates/flint-gate/ — bin: main.rs entry point, depends on core
- crates/flint-gate-client/ — lib: Rust client SDK (stub, populated in build-rust-client-sdk)

Root Cargo.toml becomes [workspace] members = ["crates/*"] with [workspace.package] for shared metadata.

Move all src/ modules into flint-gate-core/src/. Move main.rs into flint-gate/src/main.rs. Update all imports.

## Tasks
- [ ] Create crates/flint-gate-core/ with lib.rs re-exporting all modules
- [ ] Create crates/flint-gate/ with main.rs depending on flint-gate-core
- [ ] Create crates/flint-gate-client/ stub with lib.rs
- [ ] Update root Cargo.toml to [workspace] members = ["crates/*"]
- [ ] Move Cargo.lock to workspace root
- [ ] Update Dockerfile build context for new layout
- [ ] Verify cargo test --workspace passes (74 tests)
- [ ] Verify cargo clippy --workspace -- -D warnings passes
