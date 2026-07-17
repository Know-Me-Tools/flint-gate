# Tasks — add-ts-sdk-integration-tests

- [x] Create `sdks/typescript/src/__tests__/integration.test.ts` with `INTEGRATION_GATEWAY_URL` skip guard (uses `describe.skipIf`)
- [x] Add `test:integration` script to `sdks/typescript/package.json`
- [x] Implement health (`getHealth`/`getReady`), route CRUD, and API key CRUD test cases using real `FlintGateAdmin`
- [x] Verified `pnpm test:integration` collects 4 tests and skips cleanly when no fixture is running; `tsc --noEmit` passes with zero errors
