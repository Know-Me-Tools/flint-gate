# Refinement Log — add-exposure-guardrails

_Change 2/4 of `agent-gateway-exposure-operability` (Goal G3)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | No secrets; new log/bail sites carry only operator-config URLs + `server.listen`, never a token. |
| Never expose the admin server (4457) to public internet | PASS | This change *adds* an OAuth exposure fail-safe; admin posture untouched (rename regression-tested). |
| Never break existing unit tests without updating them | PASS | 403 core tests green; existing admin-posture + delegate tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | Adds `server.allow_insecure_upstream` field only; no precedence change. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 403 core tests, 0 failed (8 ignored).
- New-code coverage: 4 scheme-validation tests, 6 exposure-posture tests, over-cap
  Hydra-body deny (wiremock 70 KiB), introspect no-redirect deny, http_body unit
  tests.

## Separated security review (security-reviewer agent)

Author did not grade its own fail-safe seams. Verdict: **all three design
invariants HOLD** — no CRITICAL/HIGH. 1 MEDIUM + 2 LOW, all remediated.

### Invariants confirmed (PASS)

- **listen_is_loopback** — no false-loopback bypass; every crafted vector
  (`127.0.0.1.evil.com`, int-form `2130706433`, `[::ffff:127.0.0.1]`,
  `localhost.attacker.com`, missing port) → non-loopback → RefuseStart (fail-safe).
- **https-only scheme gate** — `https:/\evil`, `https:evil`, `ftp://`, bare host
  all refused; validation gates the exact client that gets built (no parse/normalize
  gap to reqwest).
- **body-cap streaming** — `bytes_stream()` per-chunk abort BEFORE buffering; a
  lying/absent Content-Length cannot bypass the streaming cap.
- **introspect fail-closed** — any read error → `active:false`; no path to
  `active:true` from a truncated/over-cap body.
- **admin_auth_posture rename** — no regression.
- **no secrets logged**.

### MEDIUM — introspection delegate followed HTTP redirects — FIXED

Asymmetry: the token-exchange delegate used a no-redirect client
(`Policy::none()`), but the introspection delegate reused the shared
redirect-following client. Since it POSTs the caller's token to Hydra's admin
endpoint, a compromised/misconfigured Hydra 3xx could re-POST the token to an
attacker host (same class the token-exchange path defends).

**Remediation (this change, pre-archive):** the introspection delegate now
builds a dedicated `redirect(Policy::none())` client — parity with the
token-exchange delegate. A Hydra 3xx → non-2xx → `active:false` (deny). Covered
by `delegate_does_not_follow_redirects`.

### LOW — remediated / accepted

- **L2 usize add overflow** (theoretical) → hardened with `saturating_add` in
  the body-cap check.
- **L1 prefix-match vs url::Url parse** → accepted; prefix-match on the exact
  same string handed to reqwest is sound for the http/https distinction.
- **L5 rate_limit-degrade note** → informational; the startup gate requires
  `rate_limit.enabled`, and the Redis-outage degrade is an intentional,
  separately-configured runtime tradeoff (change 1).

## Outcome

**PASS** — all constraints satisfied; the three exposure invariants hold
fail-safe; the MEDIUM redirect-asymmetry + LOW overflow were remediated and
tested before archive. Proceed to archive.
