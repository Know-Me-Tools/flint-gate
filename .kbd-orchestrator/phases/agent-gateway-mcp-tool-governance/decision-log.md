# Decision Log — agent-gateway-mcp-tool-governance

### 2026-07-06 — G1 delegate-classification mechanism
Options: gateway-side act-claim classification vs Ory Hydra claim mapper (stamp flint_kind=agent)
Decision: gateway-side act-based classification | Provenance: constraint + research (MCP authz spec, Cerbos MCP-authz, ory/hydra#2552)
Rationale: keeps the gateway a pure verifier (federate, never an IdP), covers ANY JWKS IdM not just Hydra, smaller change. Hydra claim mapper documented as an optional operator enhancement.

### 2026-07-06 — G2 agent-budget outage posture
Options: keep fail-OPEN (availability) vs fail-closed for agent scope
Decision: reuse BackendUnavailablePosture; Agent -> Deny (fail-closed) default, User/Team -> degrade | Provenance: recommendation (governance discipline)
Rationale: an over-budget agent must not slip through on a backend blip; human traffic keeps availability-first degrade.

### 2026-07-06 — G3 tool-authz metric label cardinality
Options: label by tool name vs decision-only
Decision: decision-only (allow/deny), tool name stays in DB audit | Provenance: research/design (cardinality)
Rationale: tool names are runtime/attacker-influenced; a raw-name label explodes Prometheus cardinality (same trap the delegate metric avoided with &'static str).

### 2026-07-06 — External PDP (Cerbos/OPA) vs embedded Cedar
Decision: embedded cedar-policy 4 (no external PDP) | Provenance: research (Cerbos MCP-authz architecture)
Rationale: the Cerbos reference architecture is exactly what flint-gate already does with in-process Cedar (per-tool-call check, list_tools filtering, decision logs) — no network hop, already built. Zero new dependencies this phase.

### 2026-07-06 — Ergonomics (higher-level agent-tool affordance)
Decision: out of scope this phase | Provenance: recommendation
Rationale: admin Policies tab + Cedar write-time validation exist; keep the phase on the governance core (classification + budget + metrics).
