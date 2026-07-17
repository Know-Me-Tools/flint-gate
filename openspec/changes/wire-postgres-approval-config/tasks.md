# Tasks — wire-postgres-approval-config

- [ ] Read config loading path to find where `AppState` is constructed and where to inject backend selection
- [ ] Add `approval.backend` key to config schema (default: `memory`)
- [ ] Add env var override `FLINT_APPROVAL_BACKEND` following existing env var convention
- [ ] Instantiate `PostgresApprovalStore` or `MemoryApprovalStore` based on config at startup
- [ ] Add `approval.backend: memory` to `config.test.yaml` explicitly
- [ ] Update `docs/docs/operations.md` with the new config key and migration instructions
- [ ] Remove or annotate `sessionAffinity: ClientIP` in `k8s/service-admin.yaml`
- [ ] Run `cargo test --workspace` and confirm config priority order tests pass
