# Refinement Log — add-local-exchange-metric-strict-ratelimit

_Change 2/3 of `agent-gateway-budget-and-policy-operability` (Goal G2 —
build-002 local-exchange metric + build-003 strict cross-replica mode)._
_QA gate: artifact-refiner constraint validation + separated security review._

## Constraints validated (`.kbd-orchestrator/constraints.md`)

| Constraint | Verdict | Evidence |
| --- | --- | --- |
| Never commit secrets / signing keys / prod DB creds | PASS | Metric label + config flag; no secrets. Test-only signing secret unchanged. |
| Never expose the admin server (4457) to public internet | PASS | New metric served on admin port only; strict mode only *tightens* proxy-side OAuth exposure. |
| Never break existing unit tests without updating them | PASS | 442 core tests green; 33 token-exchange tests unregressed. |
| Never change config priority order (CLI > env > YAML) | PASS | `require_shared_backend` uses plain `#[serde(default)]`; no CLI/env wiring. |

## Verification gate

- `cargo check --workspace` — clean.
- `cargo clippy --workspace --all-targets -- -D warnings` — clean.
- `cargo test --workspace` — 442 core + all sub-crates, 0 failed (8 ignored).
- Feature matrix: strict-mode + `has_shared_ratelimit_backend` tests pass under
  BOTH `--features default` (redis-l2 on) and `--no-default-features` (off).
- New-code coverage (12 new tests): metric render-all-reasons; local-exchange
  success + each of deny_verify / deny_downscope / mint_failed metered; strict
  refuse-without-shared-limiter; enforce-with-shared-limiter (feature-gated);
  ignored-on-loopback; non-strict-starts; predicate needs-both + feature;
  YAML-parse default-false; refuse-without-redis-l2-feature.

## Separated security review (security-reviewer agent)

Verdict: **correct and fail-closed as intended — no CRITICAL / HIGH / MEDIUM.**
All four invariants verified to HOLD:

- **A. Fail-closed preserved** in the `exchange()` `?`→arms restructure: every
  denial (verify / downscope / minter-absent / mint) returns the SAME error and
  issues no token; `success` recorded only on the real mint path; delegate branch
  returns before any local metering (modes stay label-disjoint).
- **B. Label safety**: `record_local_exchange(&'static str)`, all call sites are
  compile-time literals — no request-derived value reaches a label (4-value
  bounded cardinality).
- **C. Strict mode monotonic**: `require_shared_backend` can only add a conjunct
  to the `Enforce` condition (`Enforce → RefuseStart`, never the reverse);
  loopback short-circuits first (intended, not exploitable — endpoints aren't
  internet-reachable).
- **D.** No new secret/panic/unwrap on a request path; config priority untouched.

### LOW-1 — doc-vs-behavior on the compiled-out guarantee — FIXED

The reviewer found `has_shared_ratelimit_backend()`'s doc claimed strict mode
"still refuses when the feature is compiled out," but the config-only predicate
returned `true` for a Redis-configured `--no-default-features` build → it would
`Enforce` while no shared limiter actually runs (the exact under-enforcement the
flag prevents). **Remediation:** made the predicate honor the doc — it now also
requires `cfg!(feature = "redis-l2")`, so a feature-less build reports no shared
backend and strict mode genuinely refuses. Added test
`strict_shared_backend_refuses_without_redis_l2_feature_even_if_configured`
(`#[cfg(not(feature = "redis-l2"))]`) proving the refusal, and made the two
feature-dependent positive assertions feature-aware so the suite is correct in
either build.

### LOW-2 — governance lint is boot-YAML-scoped (DB routes unlinted) — NO ACTION

Pre-existing note carried from change 1 (the reviewer saw it because change 1's
governance-lint diff is also still uncommitted). Already tracked as phase debt via
the reusable `agent_governance_lint_routes()` helper + doc NOTE. Not part of this
change.

### Scope note (reviewer)

The uncommitted diff spans change 1's governance-lint code too (not yet
committed). The reviewer confirmed that code clean under the same invariants; it
is out of scope for THIS change and was already QA'd + archived under change 1.

## Outcome

**PASS** — described change verified correct + fail-closed; LOW-1 (a real
doc-vs-behavior gap in this change's own code) fixed and tested across the feature
matrix; LOW-2 is pre-existing tracked debt. Proceed to archive.
