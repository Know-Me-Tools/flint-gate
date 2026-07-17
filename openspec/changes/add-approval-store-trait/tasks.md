# Tasks — add-approval-store-trait

- [ ] Read `crates/flint-gate-core/src/approval/mod.rs` in full to map all methods and usages
- [ ] Define `ApprovalStore` trait with all 6 method signatures
- [ ] Rename `ApprovalManager` struct to `MemoryApprovalStore` and implement `ApprovalStore` for it
- [ ] Update `AppState` field type from concrete struct to `Arc<dyn ApprovalStore + Send + Sync>`
- [ ] Update all `AppState` construction sites to wrap `MemoryApprovalStore` in `Arc`
- [ ] Run `cargo test --workspace` and confirm zero regressions
