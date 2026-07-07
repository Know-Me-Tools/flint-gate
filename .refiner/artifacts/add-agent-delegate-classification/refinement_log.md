# Refinement Log — add-agent-delegate-classification

_Change 1/3 of `agent-gateway-mcp-tool-governance` (Goal G1 · build-001)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | No secrets; classification logic only. |
| Never expose the admin server (4457) to public internet | PASS | Unrelated (auth classification). |
| Never break existing unit tests without updating them | PASS | 413 core tests green; the misleading `kratos_session_flint_kind_still_trusted` test was correctly REPLACED (it codified a vuln). |
| Never change config priority order (CLI > env > YAML) | PASS | No config change. |

## Verification gate

- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 413 core tests, 0 failed (8 ignored).
- New-code coverage: act→Agent (any JWKS shape), malformed-act (7 vectors),
  Kratos-session-act, bare-client_id, well-formed-act (sub required),
  flint_kind-strip spoof (partition_jwt_claims), strip_untrusted_kind + non-object noop.

## Finding-driven scope (narrower + one new HIGH-class fix)

- **Assess/Execute finding:** `derived_kind` already classified `act`→Agent, and
  jwt_verify already surfaced `act` — so a Hydra-delegate token already classified
  as Agent, IdM-agnostically. Change 1 became verify + harden + test + doc, not new
  classification logic.
- **HIGH-class spoof vector found + fixed BEFORE archive:** both JWKS verifiers
  (jwt_verify, mcp) AND the Kratos authenticator copied untrusted upstream
  `flint_kind` into `metadata_public`, which `derived_kind` trusts — a federated
  IdP or self-service Kratos identity could forge `flint_kind: agent`/`service` and
  escalate. Fixed: `flint_kind` is stripped on ALL three federated metadata paths
  (jwt/mcp via SKIP_KEYS, Kratos via `strip_untrusted_kind`). The old test that
  *asserted* the vulnerable behavior was replaced.

## Separated security review (security-reviewer agent)

Author did not grade its own escalation seam. Verdict: **no CRITICAL/HIGH**; the
strip is complete across all production authenticators (jwt/mcp/kratos; api_key +
client_credentials set `kind` explicitly / mint gateway-signed, no attacker
channel). Two LOW findings, both remediated:

- **LOW-1 (Service re-entry asymmetry)** — a client-credentials `flint_kind:service`
  token (no `act`) downgrades to User on a JWKS round-trip (fail-safe, never
  escalation). **Documented** on `strip_untrusted_kind`.
- **LOW-2 (`act` without structural validation)** — `is_well_formed_act` accepted
  any non-empty object. **Hardened** to require a non-empty string `sub`
  (RFC 8693 §4.1), with tests.

Confirmed by review (no finding): flint_kind strip complete; Kratos escalation
resolved at the source; precedence safe (stripped flint_kind falls through to
act/User); partition_jwt_claims preserves prior trait/metadata routing;
classification alone confers no grant (Cedar policy + NHI revocation still gate).

## Outcome

**PASS** — all constraints satisfied; a HIGH-class forged-`flint_kind` escalation
was found and closed across all three federated paths before archive; both review
LOWs remediated. Proceed to archive.
