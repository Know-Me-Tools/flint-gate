# Reflection — agent-authz-control-plane

_Generated: 2026-07-04 · Phase status: **COMPLETE** (8/8 changes archived)_

## Phase Goal

Turn flint-gate from an auth *proxy* into a credible **MCP-era agent gateway** by
adding the agent-authorization control plane on top of the existing streaming
enforcement (mid-stream SSE token metering + session watchdog + AG-UI/A2UI).
Authorization-first; deliberately outside the LLM-ops bundle (semantic caching /
multi-LLM routing / multimodal), which is off-identity.

**Seed:** `.kbd-orchestrator/evolution-briefs/ai-agent-gateway-parity.md` · **Criteria:** effort-impact

## Goal Achievement

| # | Goal | Gap | Status | Evidence |
|---|------|-----|--------|----------|
| 1 | Budget enforcement + windowed rate limiting | G3 | ✅ **MET** | `add-budget-rate-limiting`; governor in-process shield + Redis Lua window counters + Postgres `usage_events` ledger; per-key/per-team minute/hour/day budgets & rate limits, block-on-threshold |
| 2 | MCP OAuth 2.1 resource-server support | G1 | ✅ **MET** | `add-mcp-resource-server`; RFC 9728 PRM, `WWW-Authenticate: resource_metadata`, RFC 8707 audience binding, PKCE S256, 403 `insufficient_scope`, no token passthrough. `mcp_e2e.rs`: 5/5 (PRM shape, tampered-sig, wrong-audience, missing-scope, valid-token) |
| 3 | Embedded policy engine + per-tool-call authz | G2 | ✅ **MET** | `add-policy-engine` (Cedar, ArcSwap hot-reload, write-time validation) + `add-per-tool-authz` (per-tool-call gate by name+params+claims, `list_tools` filtering, buffer-until-authorized streaming) |
| 4 | Human-in-the-loop approval gates | G5 | ✅ **MET** | `add-hitl-approval`; flagged tool call pauses, emits approval request over AG-UI/A2UI stream, resumes/aborts on `POST /approvals/{id}/decision` |
| 5 | Web configuration + observability UI | G4 | ✅ **MET** | `add-web-config-ui`; React+Vite SPA embedded via rust-embed 8 (SPA fallback), CRUD for routes/policies/api-keys, read-model `/analytics/*` + `/audit`, code-split Recharts analytics dashboard |
| 6 | Guardrail hook interface + authz audit trail | G6/G8 | ✅ **MET** | `add-guardrail-hook` (pluggable `PreRequestHook`, models deferred by design) + `add-authz-audit-trail` (every allow/deny/step-up/approval persisted, surfaced via Admin API + UI) |

**Goal completion: 6/6 MET (100%).**

### Success-criteria checklist (from goals.md)

- [x] MCP client authorization handshake end-to-end (PRM, WWW-Authenticate, audience, PKCE, step-up) — `mcp_e2e.rs`
- [x] Route policy authorizes an MCP tool call by name+params+claims; unauthorized tools removed from `list_tools` — `stream::processor` tests (`sse_filters_tools_list_in_data_frame`, fail-closed on malformed listing)
- [x] Per-key/per-team rolling-window token budget AND request-rate limit each block when exceeded — budget change tests
- [x] Flagged tool call pauses → AG-UI/A2UI approval prompt → resume/abort — HITL change + example
- [x] Web UI manages routes/auth/hooks/policies and shows live token/cost usage — SPA (this change)
- [x] Every authz decision written to an audit table, visible via Admin API + UI — audit-trail change + SPA audit view
- [x] Workspace green + new features ≥80% covered — `cargo check`/`clippy -D warnings`/`test --workspace`: **289 unit + 5 e2e + 1 doc, 0 failed**

## Delivered Changes (dependency order)

| Order | Change | Goal | Tasks | Archived |
|-------|--------|------|-------|----------|
| 1 | `add-budget-rate-limiting` | G3 | 8/8 | 2026-07-03 |
| 2 | `add-mcp-resource-server` | G1 | 9/9 | 2026-07-03 |
| 3 | `add-policy-engine` | G2a | 8/8 | 2026-07-03 |
| 4 | `add-per-tool-authz` | G2b | 6/6 | 2026-07-03 |
| 5 | `add-authz-audit-trail` | G6b | 5/5 | 2026-07-04 |
| 6 | `add-hitl-approval` | G5 | 7/7 | 2026-07-03 |
| 7 | `add-guardrail-hook` | G6a | 6/6 | 2026-07-03 |
| 8 | `add-web-config-ui` | G4 | 7/7 | 2026-07-04 |

## Adopted Libraries (Analyze decisions D-A01…A05)

| Gap | Verdict | Choice | Why |
|-----|---------|--------|-----|
| G2 policy engine | **ADOPT** | `cedar-policy@4` + `arc-swap@1` | Only native-Rust, Send+Sync, runtime-loadable, deterministic ABAC option (casbin not Send+Sync, oso deprecated, OPA/OpenFGA break no-sidecar) |
| G3 rate-limit/budgets | **HYBRID** | `governor@0.10` + `tower_governor@0.8` + hand-rolled Redis windows | In-process burst shield + authoritative shared Redis counters (reuse existing redis dep) + Postgres ledger |
| G1 MCP RS | **HAND-ROLL** | RFC 9728/8707/PKCE surface + reuse `jsonwebtoken@9` | No crate covers the RS/metadata role; `jwks` crate would force a jsonwebtoken v10 conflict |
| G4 web UI | **ADOPT** | `rust-embed@8` + React/Vite SPA + Recharts | rust-embed preserves single-binary shipping; SPA reuses the existing TS SDK in lockstep with utoipa OpenAPI |
| G1 OAuth2-client breadth | **DEFER** | — | NHI/RFC 8693 delegation is next-phase scope per seed brief |

## Artifact Quality Summary

| Metric | Value |
| --- | --- |
| Changes with QA (artifact-refiner + security-review) | 5/8 |
| Security-sensitive changes fully reviewed | 5/5 (all authz/budget/MCP changes) |
| CRITICAL issues caught pre-archive | 3 |
| HIGH issues caught pre-archive | 6+ |
| First-pass clean | 0/5 (every reviewed change surfaced ≥1 real defect) |
| Post-fix test suite | 289 unit + 5 e2e + 1 doc, 0 failed |

**Design intent: QA depth was concentrated on the security-critical changes (1–5,
authz/budget/MCP).** Changes 6 (HITL), 7 (guardrail-hook interface — no models),
and 8 (web UI) carry lower authz-decision risk; change 8 was gated instead by a
`typescript-reviewer` pass (2 HIGH fixed) + full-workspace green.

### Real defects caught by the QA gate (would have shipped otherwise)

- **`add-mcp-resource-server` (C1, CRITICAL):** an MCP provider configured without
  audience/issuer authenticated **open**. Fixed → fail-closed `FailingAuthenticator`
  in `build_authenticators`. RFC 8707 audience enforcement + no-token-passthrough confirmed.
- **`add-policy-engine` (C1, CRITICAL):** multi-replica hot-reload gap — `pg_notify('policies')`
  fired but the listener never reloaded the engine, so peer replicas served **stale policy**.
  Fixed → authz engine threaded into the cache-invalidation listener, retain-last-good on failure.
- **`add-policy-engine` (H1/H3, HIGH):** unvalidated `entities_json` could poison the bundle;
  admin bind defaulted to `0.0.0.0` (violates the BLOCKING "never expose admin to internet"
  constraint). Fixed → write-time entities validation + `127.0.0.1:4457` default.
- **`add-per-tool-authz` (H2, HIGH):** `list_tools` filter failed **open** on unknown response
  shapes. Fixed → fail-closed (malformed listings have tools stripped; JSON-RPC batch handled);
  unparseable non-empty args → Deny.

### Constraint checks (recurring)

All reviewed changes PASSED the blocking constraints: no secrets committed, admin
port 4457 not exposed, existing tests intact, no `unwrap`/`expect` outside tests,
parameterized SQL. **No recurring constraint violations** across changes — each
finding was change-specific.

## Technical Debt Introduced

1. **Admin API is unauthenticated (loopback-only).** The whole control plane
   (CRUD + analytics + approvals) trusts network isolation — `127.0.0.1:4457`,
   no authn on the admin router itself. Deliberate for this phase (noted in
   `add-policy-engine` H3); **full admin authn is the top follow-up.**
2. **Web UI has no RBAC** (out of scope by design) — anyone who can reach the
   admin port has full config authority. Follows directly from (1).
3. **Redis L2 optionality for budgets.** Accurate cross-replica budgets require
   Redis; with `redis-l2` disabled the fallback is a Postgres windowed
   `SUM(tokens)` check (coarser, more load). Open question from Analyze, not yet
   resolved to a hard dependency.
4. **`add-web-config-ui` archived with `--skip-specs`.** Like all 7 sibling
   changes, it carries `proposal.md` + `tasks.md` but **no OpenSpec `specs/`
   capability delta**, so strict `openspec validate` fails. The phase's changes
   are Rust/UI implementation deltas, not capability-spec deltas — a backend
   *convention* gap, not a work defect. If future phases want the `openspec
   validate` gate, changes must author `specs/<capability>/spec.md` deltas.
5. **uPlot not used.** The analytics dashboard uses Recharts for all charts;
   uPlot (planned for "dense series") was correctly skipped — bucketed
   hour/day series are low-cardinality, so uPlot would be premature (YAGNI).
   Revisit only if a raw-event, thousands-of-points view is added.
6. **`AdminError.body` captured but unused** in the SPA (LOW, from TS review);
   `useUsageSummary`/`useAudit` embed a `{}` default object in the query key
   (structurally safe under TanStack v5, but inconsistent with the
   primitive-key style used elsewhere).

## Lessons Captured (for the knowledge base)

- **Separated QA/verification pays for itself on authz code.** Every one of the 5
  security-reviewed changes surfaced a real fail-**open** defect that the
  implementer's own report missed (audience bypass, stale-policy replica, open
  `list_tools`). The pattern — *the agent that wrote the code never grades it* —
  caught 3 CRITICAL + 6 HIGH that would otherwise have shipped. Keep security-review
  mandatory on any change touching an authorization decision.
- **"Fail-closed by default" must be tested explicitly, not assumed.** The recurring
  root cause was safe-looking code that degraded to *allow* on malformed/unknown
  input. Every authz path now has a `degrades_to_deny_not_open` style assertion.
- **Dependency-conflict analysis up front saved a rework loop.** Pinning
  `jsonwebtoken@9` and rejecting the `jwks` crate (which pins ^10) in Analyze
  avoided a mid-execute version war; likewise choosing Cedar over casbin on the
  Send+Sync constraint.
- **Single-binary discipline shaped the UI stack.** rust-embed (not ServeDir) +
  reusing the TS SDK kept the whole gateway one binary and the SPA in lockstep
  with the utoipa OpenAPI — a constraint worth stating explicitly next time.
- **KBD-owned per-task apply loop held.** Driving OpenSpec one task/turn via
  `/kbd-apply` (not bare `/opsx:apply`) kept `progress.json`, the waypoint, and
  hooks in sync across the whole phase.

## Recommended Next Phase

**`agent-identity-and-delegation`** (NHI / RFC 8693 token-exchange) — the workstream
explicitly deferred here (D-A05 + goals.md out-of-scope). Now that the MCP
resource-server surface and the Cedar policy engine exist to build on, the next
strategic increment is **non-human-identity + delegation**:

1. **OAuth 2.0 Token Exchange (RFC 8693)** — `act`/`may_act` delegation so an
   agent can act on-behalf-of a user with a downscoped token, not the user's raw
   credential (confused-deputy prevention already half-built via no-passthrough).
2. **Admin API authentication** *(pull forward from tech-debt #1 — do this first;
   it gates safely exposing the control plane beyond loopback and unblocks any
   remote/multi-operator use of the new web UI).*
3. **OAuth2 client-credentials + token introspection breadth** (RFC 7662) for
   service-to-service agent identity.
4. **Workload identity / NHI lifecycle** — issue, rotate, and revoke agent
   identities as first-class principals in Cedar policies.

Stay authorization-first; still avoid the LLM-ops bundle. Resolve the open
question on Redis-L2-as-hard-dep for budgets before scaling to multi-replica.
