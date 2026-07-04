# add-nhi-cedar-principals

## Why
Agent / workload (non-human) identities cannot yet be named as **distinct
principals** in a Cedar policy — the engine types every principal as
`User::"<id>"`. So policies can't express "agents may call X, users may not," and
there is no issue/rotate/revoke lifecycle for NHI (Goal G4).

## What Changes
(D-B04, operator-confirmed **distinct-type** modeling):
1. **Distinct Cedar principal types** — add `Agent::` / `Service::` alongside
   `User::` by threading a principal **entity-type** (not just id) from `Identity`
   through `authz/engine.rs::authorize`. `make_uid(type_name, id)` is already
   generic, so this is a targeted thread-through.
2. **Identity kind** — add a `kind` (user | agent | service) to `auth/identity.rs::Identity`,
   derived from the auth provider / token claims (e.g. a `client_id`/`act` claim
   or provider type ⇒ agent/service).
3. **NHI lifecycle** — issue / rotate / revoke agent identities, persisted and
   surfaced via the Admin API + web UI + authz audit; **revocation takes effect on
   the next authorize** (fail-closed).

## Design
- `authz/engine.rs`: `authorize(principal_type, principal_id, action, resource, ctx)`
  — principal type from the caller's `Identity.kind` → `User`/`Agent`/`Service`
  Cedar entity type. `tool_authz.rs` passes the resolved type through.
- `Identity.kind` enum; mapping rules in the auth pipeline (client-credentials and
  `act`-claim tokens ⇒ agent/service).
- Lifecycle store (`agent_identities` table): id, kind, status (active/revoked),
  rotated_at; Admin endpoints `GET/POST /agent-identities`, revoke; UI page; every
  issue/rotate/revoke written to the authz audit trail.

## Depends on
- `add-token-exchange` + `add-client-creds-introspection` (they create the agent /
  service identities this change names as Cedar principals). Built **last**.

## Scope
IN: distinct `Agent::`/`Service::` Cedar principal types, `Identity.kind`, NHI
lifecycle store + Admin API + UI + audit, revocation-takes-effect semantics, tests.
OUT: cross-org federation of NHI; attribute-based NHI hierarchies beyond kind;
external workload-identity providers (SPIFFE/SPIRE) — future.

## Tasks
- [ ] Thread principal entity-type through `authz/engine.rs::authorize` (Agent/Service/User); update `tool_authz.rs` call sites
- [ ] Add `Identity.kind` (user|agent|service) + derivation rules (client_id / act-claim / provider ⇒ agent/service)
- [ ] NHI lifecycle store (`agent_identities`): issue / rotate / revoke; revocation enforced on next authorize (fail-closed)
- [ ] Admin API + web UI for agent identities; write every issue/rotate/revoke to the authz audit trail
- [ ] Tests: Agent principal allowed by an agent-scoped policy but User denied (and vice-versa); revoked identity denied on next authorize; `degrades_to_deny`
- [ ] Docs: NHI model + policy examples; `cargo check/clippy/test --workspace` green
