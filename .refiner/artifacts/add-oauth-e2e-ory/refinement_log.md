# Refinement Log — add-oauth-e2e-ory

_Change 4/4 (final) of `agent-gateway-exposure-operability` (Goal G4 · build-004)._
_QA gate: artifact-refiner constraint validation. Security review skipped per the
`kbd-execute` policy (compose + Playwright + docs only — no Rust source, no
auth-logic seam) after a targeted self-check of the two security-relevant items._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Only well-known **test** values in `config.smoke.yaml` (`flint-e2e-secret`, `smoke-hydra-system-secret`, `smoke-test-secret`) for an isolated compose network; none are real credentials, none appear in `crates/` or `config.example.yaml`. |
| Never expose the admin server (4457) to public internet | PASS | Ports are host-mapped for local E2E only (docker-compose.smoke.yml); not a production manifest. |
| Never break existing unit tests without updating them | PASS | Rust workspace untouched by this change — 406 core tests still green; existing `smoke.spec.ts` unaffected. |
| Never change config priority order (CLI > env > YAML) | PASS | No config-precedence change; `config.smoke.yaml` is a new mounted file, not a code change. |

## Targeted security self-check (change is not auth-logic)

- **`allow_insecure_upstream: true`** appears ONLY in `config.smoke.yaml` (marked
  "NOT for production") — never in `config.example.yaml` (the prod template) or
  any Rust default. The http:// Hydra upstreams are reachable only inside the
  isolated smoke network. No production plaintext-upstream regression.
- **Test secrets are smoke-isolated** — not referenced by crates or the prod
  example config; a leak of these known test values has no impact.
- **E2E is opt-in** (`E2E_OAUTH=1`) and self-skips otherwise, so a UI-only smoke
  run is unaffected.

## Verification gate

- `docker compose -f docker-compose.smoke.yml config` — valid.
- `config.smoke.yaml` — valid YAML, wires token-exchange (delegate) +
  introspection (delegate + auth) + client-credentials + rate-limit against Hydra.
- `tsc --noEmit` on `oauth.spec.ts` — clean; `playwright test --list` — 9 tests
  across 2 files (3 UI smoke + 6 OAuth).
- `cargo test --workspace` — 406 core tests, 0 failed (change touched no Rust).

## Delivered

- Ory Hydra (postgres-backed, JWT access tokens, migrate→serve→seed-client) added
  to the smoke stack; `config.smoke.yaml` wiring the gateway against it.
- `oauth.spec.ts`: happy-path (subject bootstrap, RFC 8693 delegate exchange,
  authenticated introspect) + fail-closed denials (unauth introspect 401,
  actor_token 400, over-rate 429). Deterministic (no timeout waits; concurrent
  burst; `--wait` on healthchecks).
- `web/e2e/README.md`: CI runbook (compose up `--wait` → run → `down -v`) +
  runtime-cost note; documents that the Hydra-outage deny paths stay in the Rust
  unit tests (deterministic) rather than a flaky live E2E.

## Outcome

**PASS** — all constraints satisfied; smoke-only insecure config + test secrets
are isolated from production; the E2E is deterministic and opt-in. Proceed to
archive.
