# Refinement Log — add-oauth-shared-ratelimit

_Change 1/4 of `agent-gateway-exposure-operability` (Goal G1)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | No secrets; the rate key is a SHA-256 hash, raw credential never logged. |
| Never expose the admin server (4457) to public internet | PASS | Change is on the proxy-port `/oauth/*` only. |
| Never break existing unit tests without updating them | PASS | 389 core tests green; existing OAuth/introspect tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | Adds `oauth.on_backend_unavailable` field only; no precedence change. |
| Parameterized SQL | N/A | No SQL in this change. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 389 core tests, 0 failed (8 ignored: live-Redis).
- New-code coverage: fail-closed decision surface (`posture_response`), caller-key
  derivation (client_id-preference + header fallback + anon), config-default
  posture, + 2 `#[ignore]`d live-Redis over-window/isolation tests.

## Separated security review (security-reviewer agent)

Author did not grade its own fail-open seam. Verdict: **PASS on the primary
security goal (fail-closed)**; 1 HIGH + 1 MEDIUM + 1 LOW found, all remediated
before archive.

### HIGH — header-rotation bypass defeated the limit as a brute-force control — FIXED

The rate key was `SHA-256(raw Authorization header)`, but the client-credentials
grant reads `client_id`/`client_secret` from the **form body**. An attacker could
rotate a dummy Authorization header (or omit it) to mint a fresh window each
request while brute-forcing `client_secret` in the body — so the limit did not
bind the client-credentials guessing surface (the stated G1 purpose).

**Remediation (this change, pre-archive):** `caller_key` now prefers the
authenticated `client_id` (form or HTTP Basic, via `extract_client_credentials`),
namespaced `cid:`; falls back to `hdr:`-hashed header, then `anon`. Both handlers
pass the extracted `client_id`. Same `client_id` now keys identically regardless
of header churn. Covered by `caller_key_binds_to_client_id_regardless_of_header`.

### MEDIUM — non-UTF8/absent credential collapsed callers into a shared `anon` bucket — MITIGATED

The client_id-preference fix means client-credentials callers (who supply a
`client_id` in the body) no longer land in `anon`; only fully unauthenticated
traffic shares it, backed by the per-replica IP governor. Stale module doc
corrected to describe the client_id-first keying.

### LOW — `enabled && per_second == 0` → ceiling 1 (silent near-total lockout) — FIXED

Startup guard in `main.rs` now `anyhow::bail!`s when `oauth.rate_limit.enabled`
with `per_second == 0`, with a clear remediation message — fail fast rather than
silently deny after one request/window.

### Confirmed correct (no finding)

- **Fail-closed seam PASS** — introspect posture hardcoded `Deny` in `main.rs`
  (not config-overridable); a Redis error is the ONLY `BackendUnavailable` path
  (no `unwrap`/`?` that would 500 or silently allow); `posture_response` maps
  Deny→503, over-window→429.
- **No key collision** — `flint:ratelimit:` vs `flint:budget:` namespace +
  `token:`/`introspect:` endpoint prefix fully isolate the counters.
- **No secret logged** — only the static `endpoint` label + `RateLimitError`
  display are logged; the caller key is a hash.

## Outcome

**PASS** — all constraints satisfied; the HIGH + MEDIUM + LOW were all remediated
and tested before archive; the primary fail-closed goal held clean. Proceed to
archive.
