# Decision Log — agent-governance-completeness-and-policy-authoring

### 2026-07-07 — Build-vs-adopt (all 3 goals)
Decision: BUILD/wire in-tree; ZERO new dependencies | Provenance: research (Assess + registries N/A)
Rationale: every goal extends existing primitives (agent_governance_lint_routes, compile_and_validate, CedarBundle::from_records merge, the /policies CRUD + React admin kit). No crate/npm candidate scored adopt/adapt.

### 2026-07-07 — D1 G1 reload-time governance error model
Decision: lint merged set at startup (bail!-under-strict, pre-serve) AND on hot-reload (WARN always; strict -> reject-route + retain-last-good, never terminate) | Provenance: Assess (reload path has no bail!; rebuild-or-retain only) + Cedar default-deny/skip-on-error
Rationale: a live process can't exit on a background NOTIFY; reject-and-retain is the fail-closed analog of bail! for a running gateway, matching the codebase's existing reload discipline.

### 2026-07-07 — D3 G2 sugar reload persistence
Decision: store validated sugar_policies as an immutable overlay on AuthzEngine; concatenate DB records ++ sugar in every build/reload path; keep config the source of truth (NOT written to authz_policies rows) | Provenance: Assess (reload_from_database is DB-only -> drops sugar on first reload)
Rationale: overlay keeps policy ownership clear + avoids a migration; CedarBundle merge already accepts a combined slice; sugar is process-lifetime-immutable so one stored Arc<Vec<PolicyRecord>> suffices.

### 2026-07-07 — D4 G2 precedence + guard removal
Decision: rely on Cedar forbid-overrides-permit for cross-source conflicts (no custom precedence); remove the db+sugar refuse-start guard once D3 lands; test permit/forbid conflict matrix | Provenance: Cedar Security ref (forbid=Deny, formally verified)
Rationale: deny-wins is a Lean-proven Cedar guarantee; custom precedence would be redundant and riskier.

### 2026-07-07 — D5 G3 admin-UI builder write target + injection
Decision: new admin endpoint {agent,allow,deny} -> compile_and_validate (same 400-gate) -> persist as sugar overlay; UI follows AgentIdentities/Routes patterns; NO raw-Cedar bypass for tool-scopes | Provenance: Cedar Security ref (string-concat injection breakout) + Assess (no admin surface today)
Rationale: an admin endpoint makes operator input attacker-adjacent; the existing allowlist-charset compile_and_validate is the required (and already-reviewed) mitigation and must be the only path. Depends on G2 (build order G1->G2->G3).
