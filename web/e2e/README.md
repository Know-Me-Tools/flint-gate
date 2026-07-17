# Flint Gate E2E

Playwright end-to-end tests. Two suites:

| Spec | Needs | Default |
| --- | --- | --- |
| `smoke.spec.ts` | web admin UI (`:5173`) | runs in the standard smoke run |
| `oauth.spec.ts` | full smoke stack **incl. Ory Hydra** | **opt-in** via `E2E_OAUTH=1` |

`oauth.spec.ts` self-skips unless `E2E_OAUTH=1`, so a UI-only run never fails for
lack of Hydra.

## OAuth E2E against a real Ory Hydra

The suite exercises the authenticated OAuth surface end-to-end against the
`oryd/hydra` reference AS wired in `docker-compose.smoke.yml`:

- **happy path** — client-credentials subject bootstrap (Hydra), RFC 8693
  `/oauth/token` exchange delegated to Hydra (relays a Hydra-minted token), and
  authenticated RFC 7662 `/oauth/introspect` (`active: true`);
- **fail-closed denials** — unauthenticated introspect → `401`, a present
  `actor_token` → `400 invalid_request`, an over-rate burst → `429`.

Hydra transport / non-2xx / redirect deny paths are covered deterministically by
the Rust unit tests (`delegate_fails_closed_on_*`, incl. the no-redirect 302
guard) and are intentionally **not** re-run as a flaky live-outage E2E.

### CI invocation (compose up → wait-healthy → run → teardown)

The compose file has healthchecks and ordered `depends_on`
(`postgres` → `hydra-migrate` → `hydra` → `hydra-seed` → `flint-gate` → `web`),
so a single `--wait` brings the stack up only once every service is healthy — no
arbitrary sleeps.

```bash
# 1. Bring the stack up and BLOCK until every healthcheck passes.
docker compose -f docker-compose.smoke.yml up -d --wait

# 2. Run the OAuth E2E (host ports: gateway 4456, Hydra 4444/4445).
E2E_OAUTH=1 pnpm --dir web exec playwright test oauth.spec.ts

# 3. Tear down (remove volumes so the next run re-seeds cleanly).
docker compose -f docker-compose.smoke.yml down -v
```

Override the endpoints if the host ports differ:
`GATEWAY_URL`, `HYDRA_PUBLIC_URL`.

### Determinism

No timeout-based waits. Each test asserts on a concrete HTTP response; the
over-rate test fires its burst with `Promise.all` (concurrent) so the governor's
burst bucket is exhausted by request count, not elapsed time. `--wait` gates the
run on healthchecks rather than a fixed delay.

### Runtime cost

The OAuth suite adds the Ory Hydra bring-up: a Postgres schema migration
(`hydra-migrate`) + Hydra readiness + client seed. Budget **~30–60 s** of stack
warm-up (dominated by the migration + Hydra readiness `start_period`), then the 6
API tests run in a few seconds. This is why the suite is opt-in (`E2E_OAUTH=1`)
rather than part of every smoke run.
