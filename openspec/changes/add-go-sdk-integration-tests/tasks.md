# Tasks — add-go-sdk-integration-tests

- [x] Create `sdks/go/integration_test.go` with `//go:build integration` tag and env-var client setup
- [x] Implement `TestIntegration_Health` (GET /health → status "ok") + `TestIntegration_Ready`
- [x] Implement `TestIntegration_Routes` (CreateRoute → GetRoutes → GetRoute → DeleteRoute round-trip + idempotent delete)
- [x] Implement `TestIntegration_APIKeys` (CreateAPIKey → ListAPIKeys → DeleteAPIKey; assert secret returned once)
- [x] Verified `go vet -tags integration .` passes; live run verified in CI step (change 4)
