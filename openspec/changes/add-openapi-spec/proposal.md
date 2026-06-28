# add-openapi-spec

## Summary
Add utoipa OpenAPI 3.1 generation to the admin API.

## Design
Add utoipa 5.x with axum_extras, chrono, uuid features to flint-gate-core. Annotate all admin handlers with #[utoipa::path(...)]. Derive ToSchema on response types. Serve /openapi.json and Swagger UI at /docs on admin port.

Library: adopt utoipa 5.x (library-candidates.json D2).

## Depends on
- convert-to-workspace (crate structure must exist)

## Tasks
- [ ] Add utoipa + utoipa-axum + utoipa-swagger-ui to flint-gate-core Cargo.toml
- [ ] Derive ToSchema on admin response types (routes, api-keys, signing-keys, cache-stats)
- [ ] Annotate admin handlers with #[utoipa::path(...)] 
- [ ] Create OpenApi struct with all paths registered
- [ ] Add /openapi.json and /docs routes to admin router
- [ ] Verify cargo test --workspace passes
