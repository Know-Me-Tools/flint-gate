# Goals — cedar-policy-versioning-and-rollback

_Seeded from `cedar-policy-authoring-ux/reflection.md` → "Recommended Next Phase → Option A" (auto-seeded). The prior phase delivered the full Cedar policy authoring loop: validate, observe hot-reload errors, author with inline feedback, simulate decisions. The missing piece is **reversibility** — once a policy is written and reloaded, operators have no way to recover the previous version if the change has unintended effects._

## Phase Goal

**Make Cedar policy changes reversible** by delivering: a policy version history table (`cedar_policy_versions`), a `GET /policies/{id}/history` endpoint returning ordered revisions, a `POST /policies/{id}/rollback` action that restores a prior version and triggers a hot-reload, and a version history panel in the admin UI. This completes the Cedar authoring loop:

> validate → simulate → write → watch reload status → **rollback if needed**

Still **authorization-first**; still **federate any JWKS-capable IdM (Ory reference), never an IdP**; multi-approver approval and step-up auth stay out of scope for this phase.

**Seeded from:** `cedar-policy-authoring-ux` reflection · **Criteria profile:** operator-safety (make policy deployments reversible)

## Known Starting Point (VERIFY + refine in Assess)

From the prior phase reflection and codebase state:

- **Cedar engine**: live with hot-reload, write-time validation, `ReloadStatus`, `AdminEvent` SSE.
- **Policy storage**: `gate_cedar_policies` table in Postgres; upsert via `POST /policies`, delete via `DELETE /policies/{id}`. No history table yet.
- **Admin UI**: policy editor with inline errors, validate button, valid indicator, hot-reload error banner. No history or rollback UI.
- **`POST /policies/simulate`**: live; evaluates against current policy bundle.
- **No version history**: once a policy is overwritten via upsert, the prior text is gone. No audit trail of policy text changes (distinct from the authorization decision audit trail in `gate_authz_audit`).

## Goals (build order — dependency-aware; refined by Assess/Spec)

1. **Policy version history schema** *(data layer prerequisite).*
   Add `cedar_policy_versions` table: `(id SERIAL PK, policy_id TEXT NOT NULL REFERENCES gate_cedar_policies(id) ON DELETE CASCADE, policy_text TEXT NOT NULL, schema_json JSONB, entities_json JSONB, written_by TEXT, written_at TIMESTAMPTZ DEFAULT now(), version_num INT NOT NULL)`. On every upsert to `gate_cedar_policies`, insert a row into `cedar_policy_versions` (trigger or application-level). `version_num` is monotonically increasing per `policy_id`.
   (HIGH — all other goals depend on this.)

2. **Admin API: policy history endpoint** *(authoring visibility).*
   `GET /policies/{id}/history` returns `{ policy_id, versions: [{ version_num, policy_text, schema_json, entities_json, written_by, written_at }] }` ordered by `version_num DESC` (newest first). Default limit 20, pagination via `?offset=N&limit=N`.
   (HIGH — unblocks rollback and the UI history panel.)

3. **Admin API: policy rollback endpoint** *(reversibility).*
   `POST /policies/{id}/rollback` with body `{ version_num: N }`: loads the target version row, upserts it back into `gate_cedar_policies` (which also creates a new version entry for the rollback), triggers Cedar hot-reload, returns `{ status: "rolled_back", policy_id, from_version, to_version }`. Returns 404 if version not found, 422 if the rolled-back policy fails Cedar validation (fail-closed — never write an invalid policy to the live table).
   (HIGH — the core reversibility primitive.)

4. **Admin UI: policy version history panel** *(operator UX).*
   In the policy editor modal/page, add a collapsible "Version History" section that lists the last N versions with `written_at`, a "View" action (shows the policy text in a read-only textarea), and a "Rollback to this version" button that calls `POST /policies/{id}/rollback`. Show a confirmation dialog before rollback. After rollback, the editor refreshes to show the restored text.
   (MEDIUM — depends on G2 + G3.)

## Explicitly Out of Scope (this phase)

- Quorum / multi-approver approval policies.
- Step-up authentication flows.
- Policy diff view (text diff between versions) — useful but deferred.
- Global policy rollback (roll back all policies to a point-in-time snapshot).
- LLM-ops bundle (semantic caching, multi-LLM routing).
- Full OAuth2 AS / IdP federation.

## Open Questions (resolve during Assess/Analyze)

- **Trigger vs. application-level versioning**: should the `cedar_policy_versions` insert happen in a Postgres trigger (atomic with the upsert, no app-layer change needed) or in the Rust application code (alongside the existing `upsert_policy` handler)? Trigger is safer for consistency; app-layer is easier to test.
- **`written_by` attribution**: should we capture the admin credential (JWT subject, session ID, or API key client_id) that performed the write? The current upsert handler doesn't thread caller identity through.
- **Version retention policy**: keep all versions forever, or cap at N (e.g., 100) per policy_id? Cap avoids unbounded table growth; forever is simpler.
- **Rollback re-validation**: rollback calls `validate_policy()` before writing. Should it also call `simulate` against a known test request to verify behavioral intent? Probably not — that's operator responsibility.
- **Admin UI framework for diff**: if a diff view is added in a future phase, what library? `diff` crate server-side or a JS diff library client-side?

## Success Criteria (draft — refined by /kbd-assess + /kbd-spec)

- [ ] `cedar_policy_versions` table exists with correct schema and foreign key.
- [ ] Every `gate_cedar_policies` upsert creates a corresponding `cedar_policy_versions` row with monotonically increasing `version_num`.
- [ ] `GET /policies/{id}/history` returns ordered version list; covered by integration tests.
- [ ] `POST /policies/{id}/rollback` restores prior version, triggers reload, returns structured response; returns 422 on invalid rollback target.
- [ ] Admin UI shows version history and rollback button in policy editor.
- [ ] Rollback of an invalid policy (e.g., manually corrupted version row) is rejected with 422 — fail-closed.
- [ ] Workspace green: `cargo check/clippy -D warnings/test --workspace`; new features ≥80% covered; web build green.
