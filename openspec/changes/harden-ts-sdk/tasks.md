# Tasks — harden-ts-sdk

- [ ] Read `sdks/typescript/src/client.ts` in full to understand current options, `doJSON`, and error types
- [ ] Add `TokenProvider` type (`() => Promise<string>`) to `client.ts`
- [ ] Add static token adapter for backwards compatibility
- [ ] Add retry-on-429 to `doJSON` (max 3 retries, 500ms initial, factor 2, ±20% jitter)
- [ ] Add `isRateLimited`, `isUnauthorized`, `isApprovalRequired` helper functions
- [ ] Read `sdks/typescript/src/stream.ts` and add SSE reconnect loop (max 5 retries, exponential backoff)
- [ ] Write unit tests covering 429 retry, error helpers, and SSE reconnect (mock fetch/server)
- [ ] Run `pnpm test` in `sdks/typescript/` and confirm passing
