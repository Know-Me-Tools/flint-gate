# Proposal: add-go-sdk-integration-tests

## Why

`sdks/go/client_test.go` covers SSE parsing, HTTP plumbing, and middleware
exclusively using `httptest.NewServer`. No test exercises the real admin API
endpoint of a live flint-gate process. Behavioral correctness of `GetHealth`,
routes CRUD, and API key CRUD against an actual database-backed server is
currently untested. This gap means a regression in the server's route
serialization or auth header handling could go undetected by the Go SDK's test
suite.

## What Changes

- **Create** `sdks/go/integration_test.go` with build tag `//go:build integration`
  so it is excluded from `go test ./...` unit runs
- Integration tests read `INTEGRATION_GATEWAY_URL` (default `http://localhost:4457`)
  and a static `INTEGRATION_ADMIN_TOKEN=integration-test-token`
- Test functions: `TestIntegration_Health`, `TestIntegration_Routes`,
  `TestIntegration_APIKeys`
- Each test cleans up resources it creates via `t.Cleanup`
- No new dependencies — uses only the existing `sdks/go` module and stdlib
