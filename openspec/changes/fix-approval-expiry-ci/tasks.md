# Tasks — fix-approval-expiry-ci

- [x] Read `docker-compose.test.yml` and confirm or add short-TTL config (ttl_seconds: 5, janitor_interval_seconds: 1)
- [x] Read `config.test.yaml` and confirm approval TTL settings are present
- [x] Read `sdks/typescript/src/__tests__/integration.test.ts` to check for existing expiry test
- [x] Add TypeScript approval expiry integration test if absent (register → sleep 6s → assert denied)
- [x] Run `pnpm test:integration` locally to confirm passing (or note stack requirement)

## Notes

All conditions were already met — no code changes required:
- `config.test.yaml` has `approval.ttl_seconds: 5` and `janitor_interval_seconds: 1` (lines 59–63)
- `docker-compose.test.yml` mounts this file via volume bind and `FLINT_GATE_CONFIG` env var
- TypeScript expiry test exists at `integration.test.ts:222–273`:
  - Creates policy with `@require_approval`, triggers via streaming request
  - Sleeps 8_000ms (> TTL 5s + janitor 1s)
  - Asserts `listApprovals()` is empty after janitor sweep
  - 30_000ms test timeout (well within CI 120s limit)
- `pnpm test:integration` requires the docker-compose stack; cannot run without it
