# Tasks — harden-go-sdk

- [ ] Read `sdks/go/client.go` in full to understand current `Options`, `doJSON`, and error types
- [ ] Add `TokenSource` interface (`GetToken(ctx context.Context) (string, error)`) to `client.go`
- [ ] Add `StaticTokenSource` adapter for backwards-compatible static token strings
- [ ] Add retry-on-429 to `doJSON` (max 3 retries, 500ms initial, factor 2, ±20% jitter)
- [ ] Add `IsRateLimited`, `IsUnauthorized`, `IsApprovalRequired` helper functions
- [ ] Read `sdks/go/stream.go` and add SSE reconnect loop (max 5 retries, exponential backoff)
- [ ] Write unit tests covering 429 retry, error helpers, and SSE reconnect (mock HTTP server)
- [ ] Run `go test ./...` in `sdks/go/` and confirm passing
