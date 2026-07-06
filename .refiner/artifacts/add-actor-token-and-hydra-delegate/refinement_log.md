# Refinement Log — add-actor-token-and-hydra-delegate

_Change: final change of `agent-gateway-hardening-and-exposure` (Goal G4)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | No secrets touched; delegate `token_url` is operator config, not a credential. |
| Never expose the admin server (4457) to public internet | PASS | Change is on the proxy port's `/oauth/token` only; admin surface untouched. |
| Never break existing unit tests without updating them | PASS | 378 core tests green; existing exchange fail-closed tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | No config-precedence change; adds `delegate_to_hydra`/`hydra_token_url` fields only. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 378 core tests, 0 failed (6 ignored).
- New-code coverage: actor_token reject (both directions) + delegate forward +
  4 delegate fail-closed paths (transport / non-2xx / bad-JSON / redirect).

## Separated security review (security-reviewer agent)

Author did not grade its own fail-open seams. Verdict: **PASS** — no
unremediated CRITICAL/HIGH.

### HIGH — delegate client followed HTTP redirects (subject_token exfiltration) — FIXED before archive

The `HydraDelegate` reused the shared `reqwest::Client` (default redirect policy
= follow up to 10). A compromised/tricked Hydra returning a 3xx to an attacker
host would have the gateway re-POST the RFC 8693 form — including the caller's
raw `subject_token` — to that host. Same threat class the JWKS client already
guards against (`jwks.rs` sets `Policy::none()`).

**Remediation (this change, pre-archive):** the delegate now builds a dedicated
`reqwest::Client` with `redirect(Policy::none())` (`main.rs`). A Hydra 3xx now
surfaces as a non-2xx → `ExchangeError::MintFailed` (deny). Documented on
`delegate_exchange_to_hydra`; covered by `delegate_fails_closed_on_hydra_redirect`
(302 + `location: attacker` header → deny).

### LOW — delegate mode drops gateway downscope / `flint_kind` / `act` stamping — ACCEPTED (by design)

In delegate mode Hydra owns RFC 8693, so the gateway-local verify/downscope/
escalation-deny + `flint_kind=agent` stamp are intentionally skipped. This is a
federation seam, not a bypass of a gateway-kept guarantee; the gateway-local
path retains all guards. Operator-config risk (Hydra `aud` quirk, ory/hydra#3723)
is documented in `config/types.rs` and `config.example.yaml`. Delegated tokens
not carrying `flint_kind` is noted as intended ("Hydra owns the token").

### LOW — unbounded relay of Hydra response body — ACCEPTED

`resp.json::<Value>()` has no explicit body cap; same trust boundary as any
upstream call, gated behind operator-configured `token_url`. No injection risk
(re-serialized by axum, not templated).

### Confirmed correct (no finding)

- actor_token reject ordered FIRST in `exchange()` — before both the delegate
  branch and local mint; trim-then-`is_empty` cannot let a real value pass as
  "empty".
- No subject_token/actor_token bytes logged.
- `.form()` url-encodes — no form/header/URL injection from attacker-controlled
  values.
- Gateway-local exchange guarantees (subject verify, downscope, escalation deny)
  unregressed.

## Outcome

**PASS** — all constraints satisfied; the single HIGH was remediated and tested
before archive; residual LOWs are documented by-design federation semantics.
Proceed to archive.
