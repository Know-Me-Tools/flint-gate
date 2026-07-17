# Proposal: add-ts-sdk-integration-tests

## Why

`sdks/typescript/src/__tests__/admin.test.ts` mocks the underlying
`adminRequest` method with `vi.fn()`. No test exercises the real fetch path
against a live gateway. The TypeScript `FlintGateAdmin` client could have a
wrong URL construction, a broken JSON serialization, or an incorrect header
and the unit tests would still pass. Integration tests against the live fixture
close this gap and mirror the Go SDK coverage.

## What Changes

- **Create** `sdks/typescript/src/__tests__/integration.test.ts` using vitest;
  tests skip at `describe` level when `INTEGRATION_GATEWAY_URL` is unset
- **Add** `test:integration` script to `sdks/typescript/package.json`:
  `vitest run --reporter=verbose integration.test`
- Test coverage: health check, routes CRUD, API keys CRUD (mirrors Go tests)
- Uses the real `FlintGateAdmin` with a real `fetch` call to the fixture
- No new npm dependencies
