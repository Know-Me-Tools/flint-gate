# Assessment — beta-release-readiness

_Generated: 2026-07-09_
_Sycophancy correction: applied — findings reflect actual codebase state, not project momentum_

---

## Verdict

**Not ready for external beta as-is.** There are three hard blockers that
would cause real operational incidents in customer deployments. Eleven
further gaps are serious enough to warrant disclosure or a fix before beta.
Twelve gaps are acceptable risks that can be carried as known-issue disclosures.

This is a capable proof-of-concept that has hit a quality ceiling common in
rapid-iteration builds: the core engine is sound, but the operational envelope
(multi-replica correctness, admin surface hardening, observability, operator
documentation) has been outpaced by feature addition. That gap is closable in
3–5 weeks of focused work.

---

## Sycophancy-Correction Audit

Before findings, the patterns that would produce a false-positive "ready"
assessment if left uncorrected:

| S-Code | Pattern | Where it could surface |
|--------|---------|----------------------|
| S-01 | Approval-seeking (celebrating prior KBD phases) | Omitted — phases are irrelevant to beta readiness |
| S-03 | Scope expansion (listing all features as strengths) | Config file length ≠ production coverage |
| S-05 | Severity minimisation ("minor gap", "follow-up") | Cross-replica approval is described as a "follow-up" in comments but is a production blocker |
| S-07 | False completeness ("all auth paths are covered") | WebSocket path lacks tool-authz and approval wiring |
| S-08 | Risk laundering ("acceptable for beta") | Applying only to gaps I can independently justify, not to flatter |

---

## Blocker Gaps (must fix before any external beta)

### B-1 — Cross-Replica Approval Routing Is Silently Broken

**Severity: BLOCKER**

`ApprovalManager` is in-process, per-replica. The `POST /approvals/:id/decision`
API resolves correctly only on the replica that holds the paused stream. Any
other replica returns `NotFound`. In a multi-replica deployment (the shipped
K8s manifest has `replicas: 2` with no session affinity), approximately 50% of
approval decisions will fail with a cryptic 404. This is acknowledged in
`main.rs` at line ~862 but only emits a `warn!` if `REPLICA_COUNT > 1` is set
in the environment — a non-standard env var that no operator will know to set.

The config.example.yaml comment says "cross-replica routing is a follow-up",
but the K8s manifest already ships with 2 replicas and no sticky routing.
A beta customer who deploys as documented will immediately have broken approvals.

**Closure:** Add explicit pod-level sticky routing in the K8s service (e.g.
`sessionAffinity: ClientIP`) OR implement a shared approval store (Redis or
Postgres row). The service-level sticky approach is the smaller change (~1
change), but it's a band-aid: once Kubernetes decides to restart a pod, the
stream is gone and the approval is orphaned regardless.

---

### B-2 — Admin API Is Unauthenticated by Default, and the Default Posture Is Not Documented as a Risk

**Severity: BLOCKER**

When `admin_auth:` is not configured, the admin API accepts any request on
the admin bind with zero authentication. The only protection is the bind
address: `admin_listen: "127.0.0.1:4457"` defaults to loopback. That is
adequate for local dev. It is inadequate the moment:

- A beta customer deploys on a VPS and binds admin to `0.0.0.0` for convenience
- A beta customer runs in a cloud environment where internal cluster IPs are
  reachable across tenants (e.g. a shared k8s namespace, AWS VPC peering)
- A Kubernetes sidecar or init container reaches the admin port from inside
  the pod

The config.example.yaml has `admin_auth:` commented out. The getting-started
guide does not mention this risk. The K8s manifest does not configure
`admin_listen` at all, which means it defaults to loopback — but the
`deployment.yaml` exposes the admin port via a service on every pod. Any pod
in the cluster can hit any other pod's admin API with no credential.

There is a startup posture guard that _warns_ when a non-loopback admin bind
is configured without `admin_auth`. But the K8s network topology bypasses
this: the bind itself is loopback, but the K8s service makes it reachable
cluster-wide.

**Closure:** Document this explicitly in getting-started.md with a red-box
warning. Add K8s NetworkPolicy to the k8s/ manifests that blocks all
inter-pod admin port traffic by default. Consider making the posture guard
refuse-to-start when it detects `KUBERNETES_SERVICE_HOST` (running in k8s)
without `admin_auth` configured.

---

### B-3 — TLS Misconfiguration Silently Falls Back to Plaintext

**Severity: BLOCKER**

When `tls.enabled: true` but cert loading fails (wrong path, unreadable cert,
malformed key), flint-gate logs a `warn!` and falls back to plain TCP. A
beta customer who configures TLS, deploys, and sees `proxy server listening`
in logs will assume TLS is active. It is not. Traffic will flow plaintext.

This fail-open behavior is correct in local dev and deeply wrong in production.
There is no mechanism to make TLS failures fatal (no `tls.require_tls: true`
config option).

**Closure:** Add a `tls.fail_open: false` config option (default to `false`,
i.e. fail-closed). When disabled, a TLS load failure should call
`std::process::exit(1)` rather than falling back. The current code is in
`main.rs:727–752`.

---

## Serious Gaps (disclose or fix before beta)

### S-1 — Schema DDL Is Inline Raw SQL Without Version Tracking

The entire Postgres schema is one string constant in `db/mod.rs` applied at
startup via `sqlx::raw_sql`. It uses `CREATE TABLE IF NOT EXISTS` and
`ALTER TABLE ... ADD COLUMN IF NOT EXISTS`. This is not a migration system —
it is a re-entrant init script. The consequences:

- No rollback capability. A column added by one version cannot be safely removed
  when rolling back under load.
- No migration history. Operators cannot tell what schema version is running.
- Renaming a column requires a manual migration outside this system.
- The `ADD COLUMN IF NOT EXISTS` pattern will silently succeed on a schema that
  has diverged in incompatible ways.

**Risk to beta:** Low if schema is stable. High if any schema change is needed
during the beta period (which is virtually guaranteed).

**Closure:** Adopt sqlx's `sqlx::migrate!` with versioned `.sql` files, or at
minimum add a `schema_version` table and refuse to start on a version mismatch.

---

### S-2 — WebSocket Path Lacks Tool-Authz and Approval

`ws_bridge` in `stream/websocket.rs` does not accept a `tool_authz` parameter
and is not wired to `ApprovalManager`. Any tool call that arrives via WebSocket
upstream bypasses Cedar authorization and the human-in-the-loop approval gate
entirely. The SSE path enforces both; the WS path enforces neither.

The pipeline.rs wiring confirms this: ws_bridge is called without the authz
arguments that the SSE processor receives.

**Risk to beta:** High if any upstream uses WebSocket (e.g. MCP over WS).
Tool-call authorization is the primary value proposition of flint-gate. A
customer who uses WS upstreams will get zero enforcement.

**Closure:** Port the tool-authz + approval wiring from `SseStreamProcessor`
to `ws_bridge`. This is ~1 change, primarily mechanical.

---

### S-3 — Agent-Governance Lint Is Off by Default; No Operator Tooling to Find Ungoverned Routes

`strict_agent_governance: false` is the default. This means a route can be
AGENT-REACHABLE (JWT-backed, an agent can present a token) with no `authorize`
hook, no token budget, and no Cedar policy — and the gateway will proxy it
through silently, with no warning unless `strict_agent_governance: true` is set.

The lint logic exists. The default posture does not enforce it. Beta customers
building agent pipelines will expect the gateway to protect by default.

**Risk to beta:** Medium. A misconfigured route passes agent traffic without
Cedar evaluation. This is a silent security gap, not a crash.

**Closure:** Change default to `strict_agent_governance: true` OR add a
startup `warn!` for every ungoverned agent-reachable route that cannot be
silenced. Do not leave this as purely opt-in enforcement.

---

### S-4 — Cedar Schema Validation Is Not Enforced at Write Time

Policies are stored with `schema_json: None` and `entities_json: None`
throughout the codebase. The `validate_policy` Admin API endpoint exists, but
it validates Cedar syntax only. It does not validate against any Cedar schema
(there is no schema configured). This means an operator can write a policy
that references `Action::"nonexistent_action"` or a type-incorrect attribute,
and it will compile but evaluate incorrectly at runtime.

The `@require_approval` annotation is the key correctness surface — a typo
(`@require_apporval`) would be silently accepted and the policy would behave
as a plain permit.

**Risk to beta:** Medium. Policy errors are silent. An operator who writes
a policy believing it enforces approval, but with a typo, gets no enforcement.

**Closure:** Define and enforce a Cedar schema for the gateway's entity model
(User, Agent, Service, Route, Action::"call_tool"). Reject policies that fail
schema validation at write time. The DB schema already has `schema_json` and
`entities_json` columns — they just need to be populated.

---

### S-5 — Approval TTL Auto-Deny Has No E2E Integration Test

The janitor (background task that auto-denies expired approvals) is unit-tested
only against the in-memory `ApprovalManager`. There is no integration test that
confirms the end-to-end behavior: a stream hangs → TTL expires → janitor fires
→ stream terminates. If the janitor has a bug, a paused stream will hang
indefinitely (or until the connection is dropped), consuming resources
permanently.

**Risk to beta:** Medium. The janitor code looks correct, but untested end-to-end
expiry paths in production are a classic source of silent resource exhaustion.

---

### S-6 — Rate Limiting Is Per-Replica With No Cross-Replica Warning

The admin rate limiter, OAuth rate limiter, and proxy rate limiter are all
per-replica in-process governors. The effective ceiling in a multi-replica
deployment scales with replica count. A Redis shared backend is supported but
optional and off by default.

The config.example.yaml mentions this but only in comments. Operators who
configure `admin_rate_limit.enabled: true` thinking they have a 5 req/s ceiling
will actually have a 5×N req/s ceiling in an N-replica deployment.

**Risk to beta:** Low to medium. Not an immediate security incident, but a
rate-limit bypass that operators will discover only when something goes wrong.

**Closure:** Emit a startup `warn!` whenever rate limiting is enabled without
a shared backend in a detected multi-replica context (use `REPLICA_COUNT` or
`KUBERNETES_SERVICE_HOST`).

---

### S-7 — No CHANGELOG or Breaking-Change Tracking

The project is at version `0.1.0` across all crates and the web package. There
is no CHANGELOG. The release workflow publishes to crates.io, npm, and Docker
Hub on any `v*` tag. If a beta customer pins to `v0.1.0` and a breaking change
is introduced in `v0.1.1` (config schema change, API response shape change),
there is no way for the customer to know without reading a git diff.

**Risk to beta:** Guaranteed friction. Beta customers in real deployments pin
versions and read changelogs.

---

### S-8 — Config Hot-Reload Has No Rollback on Invalid Config

When `config.yaml` is modified on disk and the reload produces an error
(bad YAML, invalid route config), the behavior is not clear from the code.
If the reload fails silently and retains the previous config, that is correct.
If it applies a partial config, that is dangerous. The assessment of this gap
requires checking `config/loader.rs` which was not fully read.

**Risk to beta:** Potentially high. Hot-reload that partially applies a broken
config could produce inconsistent route state under live traffic.

---

### S-9 — Flutter SDK Is Stub-Level (~155 Lines Total)

The `sdks/flutter/` directory exists with `pubspec.yaml` and a `lib/` directory,
and the release workflow attempts to publish it. The total code is ~155 lines.
A customer who reads the SDK index page and tries to use the Flutter SDK will
find an incomplete library.

**Risk to beta:** Low if Flutter is not a target. High if any mobile beta
customer is expected.

**Closure:** Remove Flutter from the release workflow and docs until the SDK
is production-complete, or explicitly label it "experimental / not ready."

---

### S-10 — Admin UI Pages Are Read-Only Displays, Not CRUD Surfaces

`AuthProviders.tsx`, `Hooks.tsx`, and `Budgets.tsx` are read-only — they
display config but provide no edit capability. A beta customer who finds the
Admin UI and tries to configure an auth provider or modify a hook via the UI
will find the UI is a viewer only. There is no feedback that these surfaces
are read-only (no "editing requires config.yaml" notice).

**Risk to beta:** Medium UX impact. The first impression will be confusion
about whether the admin UI is functional.

---

### S-11 — No Operator Runbook

The docs/ site has 4 pages: getting-started, configuration, admin-api, and
intro. There is no:
- Runbook for common failure modes (DB connection failure, Cedar reload failure,
  upstream unreachable)
- Ops guide for multi-replica deployment
- Guidance on Cedar policy authoring (no examples beyond the config.example.yaml)
- Incident response guide

Beta customers in real deployments hit operational problems and need to
self-serve. Without a runbook, every issue will require reaching out.

---

## Acceptable Beta Risks (disclose but do not block on)

| # | Gap | Justification for accepting |
|---|-----|----------------------------|
| A-1 | `schema_json`/`entities_json` always null (no Cedar schema) | Schema is optional in Cedar; fail-closed semantics are not compromised |
| A-2 | `agent_tool_policies` hot-reload requires restart | Documented in config.example.yaml; acceptable for beta |
| A-3 | OAuth introspection is local-only (no Hydra federation by default) | Opt-in feature, clearly labeled disabled |
| A-4 | Token exchange `actor_token` rejected (single-hop only) | Documented limitation in config |
| A-5 | In-process L1 cache only; Redis L2 optional | Acceptable for single-node beta deployments |
| A-6 | Audit trail is append-only with no automated retention policy | Not a correctness issue; data grows but doesn't corrupt |
| A-7 | Metrics are Prometheus-pull only; no push path | Standard ops pattern; acceptable |
| A-8 | Session watchdog polls the Kratos cache, not Kratos directly | Slightly stale session state; acceptable for beta duration |
| A-9 | No content-based guardrail (regex only, no ML classifier) | Documented capability boundary |
| A-10 | CI runs on main branch only; integration tests require docker-compose | Known, documented in ci.yml |
| A-11 | K8s manifests use `image: flint-gate:latest` (mutable tag) | Development-only manifests; documented |
| A-12 | Docs site is Docusaurus but build is not wired to CI | Low risk; docs don't affect runtime |

---

## What Is Working Well (sycophancy-corrected: only objectively verifiable claims)

- **Fail-closed posture is real and enforced.** The Cedar engine denies on any
  evaluation error. The OAuth endpoint refuses to start on non-loopback without
  guardrails. The approval TTL auto-denies. These are code-verified, not claims.

- **Unit test coverage is meaningful.** 41 files with `#[cfg(test)]` blocks.
  The Cedar engine, JWT mint/verify, approval manager, and rate limiters all
  have unit tests with non-trivial coverage.

- **Integration tests exist and run against a real stack.** The Go and TypeScript
  SDK integration tests require a live flint-gate instance. This is unusual
  quality for a pre-beta project.

- **Graceful shutdown is correctly wired.** SIGTERM and SIGINT are handled;
  in-flight requests drain before exit.

- **The admin auth posture guard exists.** Non-loopback admin bind without
  `admin_auth` emits a warning at startup. It should be a refuse-to-start, but
  the guard is present.

---

## Summary Gap Table

| ID | Category | Description | Severity | Effort to close |
|----|----------|-------------|----------|-----------------|
| B-1 | Correctness | Cross-replica approval routing silent failure | **BLOCKER** | 1–2 changes |
| B-2 | Security | Admin API reachable in k8s without auth | **BLOCKER** | 1 change + docs |
| B-3 | Security | TLS misconfiguration silently falls back to plaintext | **BLOCKER** | 1 change |
| S-1 | Ops | No schema migration system | SERIOUS | 2–3 changes |
| S-2 | Correctness | WebSocket path bypasses tool-authz and approval | SERIOUS | 1 change |
| S-3 | Security | Ungoverned agent routes: off-by-default enforcement | SERIOUS | 1 change |
| S-4 | Correctness | Cedar schema validation not enforced at write time | SERIOUS | 2 changes |
| S-5 | Testing | Approval TTL expiry has no E2E integration test | SERIOUS | 1 change |
| S-6 | Ops | Rate limits are per-replica with no multi-replica warning | MEDIUM | 1 change |
| S-7 | Ops | No CHANGELOG or breaking-change tracking | MEDIUM | 1 change |
| S-8 | Correctness | Hot-reload partial-config rollback unverified | MEDIUM | audit + 1 change |
| S-9 | UX | Flutter SDK is stub-level; release workflow publishes it | MEDIUM | 1 change |
| S-10 | UX | Admin UI read-only pages have no "read-only" indicator | LOW | 1 change |
| S-11 | Ops | No operator runbook | LOW | 2–3 changes |

---

## Recommended Path to Beta

**Phase 1 (blockers, ~1 week):** Close B-1, B-2, B-3.
- B-1: K8s `sessionAffinity: ClientIP` + documentation of the per-replica constraint
- B-2: K8s NetworkPolicy restricting admin port + refuse-to-start on K8s without admin_auth
- B-3: Add `tls.fail_open: false` option defaulting to fail-closed

**Phase 2 (serious gaps, ~2 weeks):** Close S-1 through S-5.
- S-1: Adopt sqlx migrate with versioned files
- S-2: Port tool-authz + approval to ws_bridge
- S-3: Change strict_agent_governance default or add ungoverned-route startup warning
- S-4: Define and enforce Cedar entity schema
- S-5: Add approval TTL expiry integration test

**Phase 3 (beta preparation, ~1 week):** Close S-6 through S-11.
- S-6: Multi-replica rate-limit warning
- S-7: CHANGELOG with v0.1.0 baseline
- S-8: Audit hot-reload rollback behavior, fix if partial-apply is possible
- S-9: Remove Flutter SDK from release until complete
- S-10: Read-only indicators in admin UI
- S-11: Operator runbook (key failure modes + Cedar policy examples)

Total estimated effort: 3–5 weeks for a team of 1–2.
