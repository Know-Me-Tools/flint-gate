# Goals — agent-authz-control-plane

Turn flint-gate from an auth *proxy* into a credible **MCP-era agent gateway** by
adding the agent-authorization control plane on top of its existing streaming
enforcement (mid-stream SSE token metering + session watchdog + AG-UI/A2UI
processing). Authorization-first; deliberately NOT entering the LLM-ops bundle
(semantic caching / multi-LLM routing / multimodal), which is off-identity.

**Seed:** `.kbd-orchestrator/evolution-briefs/ai-agent-gateway-parity.md`
**Criteria profile:** effort-impact

## Goals (build order — impact-weighted, dependency-aware)

1. **Budget enforcement + windowed rate limiting** — extend the existing
   `usage_events` + lifetime `MaxTokenBudget` hook into per-key/per-team,
   rolling-window (minute/hour/day) token budgets AND request-rate limits.
   Block on threshold with a clear error. (Gap G3 · fastest win, highest feasibility)

2. **MCP OAuth 2.1 resource-server support** — RFC 9728 protected-resource
   metadata, `WWW-Authenticate: resource_metadata` on 401, RFC 8414/OIDC AS
   discovery, PKCE S256 verification, RFC 8707 `resource`/audience validation,
   403 `insufficient_scope` step-up, and no token passthrough to upstreams
   (confused-deputy prevention). (Gap G1 · CRITICAL — credibility gate)

3. **Embedded policy engine + per-tool-call authorization** — evaluate an
   embedded native-Rust policy engine (Cedar core or casbin-rs, no sidecar)
   inline in the stream; authorize each MCP tool call by tool name + parameters
   + identity claims; filter unauthorized tools out of `list_tools` responses
   (the agentgateway pattern). New `PreRequestHook::Authorize` + stream-level
   tool-call gate. (Gap G2 · CRITICAL — strategic core)

4. **Human-in-the-loop approval gates** — pause a flagged tool call, emit an
   approval request over the existing AG-UI/A2UI stream, resume or abort on the
   response. Deterministic pre-action authorization. (Gap G5 · HIGH — differentiator)

5. **Web configuration + observability UI** — SPA served by the Admin server
   against the Admin API: view/create/edit routes, auth providers, hooks, and
   policies, plus live token/cost analytics from `usage_events`. (Gap G4 · HIGH)

6. **Guardrail hook interface + authorization decision audit trail** — a
   pluggable guardrail `PreRequestHook` interface (defer bundling injection/PII
   models) and an audit table capturing every allow/deny/step-up/approval,
   surfaced via Admin API + UI. (Gaps G6, G8 · MEDIUM — cross-cutting)

## Explicitly out of scope (this phase)

- Semantic caching, multi-LLM routing/failover/load-balancing, prompt
  compression, multimodal, prompt management/versioning (the LLM-ops bundle —
  off-identity; revisit only on a deliberate product pivot).
- Becoming an OAuth2 *authorization server* or adding SAML/SCIM/LDAP federation
  (Ory Hydra / Keycloak / Auth0 territory — federate with them, don't replace).
- NHI / RFC 8693 delegation and OAuth2 client-introspection breadth (roadmapped
  to the *next* phase once MCP RS + policy engine exist to build on).

## Success criteria

- [ ] MCP client authorization handshake passes end-to-end (PRM, WWW-Authenticate,
      audience validation, PKCE, step-up).
- [ ] A route policy authorizes an MCP tool call by name + params + claims;
      unauthorized tools removed from `list_tools`.
- [ ] Per-key/per-team rolling-window token budget AND request-rate limit each
      block when exceeded (test-proven).
- [ ] A flagged tool call pauses, surfaces an AG-UI/A2UI approval prompt, and
      resumes/aborts on response (proven in an example).
- [ ] Web UI manages routes/auth/hooks/policies and shows live token/cost usage.
- [ ] Every authz decision written to an audit table, visible via Admin API + UI.
- [ ] Workspace green: `cargo check --workspace`, `cargo clippy --workspace -- -D warnings`,
      `cargo test --workspace`; new features ≥80% covered.
