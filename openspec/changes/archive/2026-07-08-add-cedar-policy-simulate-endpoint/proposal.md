# Proposal — add-cedar-policy-simulate-endpoint

**Phase:** cedar-policy-authoring-ux
**Goal:** G4 (MEDIUM)
**Build position:** 4 of 4 — independent of changes 2–3; depends only on the Cedar engine.

## Problem

Operators cannot test Cedar authorization decisions without sending live traffic through the gateway. `AuthzEngine::is_authorized()` already returns a structured `Response` with `decision()` (Allow/Deny) and `diagnostics()` (reasons: set of matched policy IDs, errors), but this is not exposed via any admin endpoint.

## Solution

1. Define `SimulateRequest { principal: String, action: String, resource: String, context: serde_json::Value }` and `SimulateResponse { decision: String, reasons: Vec<String>, errors: Vec<String> }` DTOs.
2. Add `POST /policies/simulate` handler that:
   - Parses the Cedar `EntityUid` values from the string fields.
   - Constructs a Cedar `Request` (principal, action, resource, context).
   - Calls `authz_engine.is_authorized(request, entities)` with empty `Entities` (no entity graph; evaluation is policy-logic only).
   - Returns `SimulateResponse` with the Cedar `Decision` and reason/error lists.
3. No side effects, no persistence, no live traffic path.
4. Scope: live-policy-only (no caller-supplied policy override — deferred to next phase).

## Scope

- `crates/flint-gate-core/src/admin/mod.rs` — `SimulateRequest`/`SimulateResponse` types; `POST /policies/simulate` route + handler
- No changes to `AuthzEngine` itself (calls existing `is_authorized`).
- No changes to `main.rs`, config, or UI in this change (UI simulate form is future work).

## Acceptance criteria

- `POST /policies/simulate` with a valid Cedar principal/action/resource returns `{ decision: "Allow" | "Deny", reasons: [...], errors: [] }` (200).
- With an active policy that allows principal P to perform action A on resource R, simulate returns `decision: "Allow"` and `reasons` includes the matching policy ID.
- With no matching allow policy, simulate returns `decision: "Deny"` and `reasons: []`.
- Invalid `EntityUid` format in the request body returns HTTP 422 with a descriptive error.
- Rate-limited by the existing `AdminGovernorLayer`.
- 3+ new tests: allow decision, deny decision, invalid entity UID format.
