# add-agent-delegate-classification

**Phase:** agent-gateway-mcp-tool-governance · **Goal:** G1 (build-001)
**Scope:** `crates/flint-gate-core/src/auth/identity.rs` (kind derivation), docs.
**Depends on:** nothing (gates changes 2 + 3 for delegated agents).

## Assess/Execute finding (narrower than seeded)

`derived_kind` (`auth/identity.rs`) **already** classifies `act`→Agent
(`session_id.is_none() && metadata_public["act"].is_some()`), and `jwt_verify`
already collects the `act` claim into `metadata_public` with `session_id: None`.
So a Hydra-delegate token (RFC 8693 `act`) **already** classifies as Agent,
IdM-agnostically — the "gap" is real only as **verification + explicit
delegate-path tests + doc**, not new classification logic. This change therefore
hardens/tests/documents the existing gateway-side rule rather than adding it.

## Why

The per-tool-call authz engine (`authorize_tool_call`, Cedar `call_tool`,
`list_tools` filtering, audit) is already built and — per the Cerbos MCP-authz
reference architecture — industry-standard. But a **Hydra-delegate-minted token
carries Hydra's claims, not the gateway's `flint_kind=agent` marker**, so a
delegated agent may classify as `User` and **escape agent tool-policy and (change
2) agent budget**. This is the linchpin gap: classification gates governance for
delegated tokens.

## What

Classify a verified token as `PrincipalKind::Agent` **gateway-side, at
verification time, from its RFC 8693 `act` claim** — no token rewriting. A token
bearing a well-formed `act` (delegation actor) is an Agent regardless of which
JWKS IdM issued it. Precedence (confirm the exact order against the existing
`derived_kind`): a gateway-signed `flint_kind` wins first (already trusted), then
a present `act` → Agent, then the existing fallbacks.

**Decided (analysis):** do **NOT** use a Hydra-side claim mapper as the mechanism
— that would push flint-gate's identity model into every operator's Hydra config
and only cover the Hydra path, violating "federate any JWKS IdM, never an IdP."
The Hydra claim mapper is documented as an OPTIONAL operator enhancement only.

## Non-goals

- Re-stamping / re-signing delegated tokens (the gateway stays a pure verifier).
- Multi-hop `actor_token` (still rejected; single-hop `act` only).

## Fail-closed requirement

An unverifiable / malformed `act` must NOT silently promote to Agent (nor crash);
ambiguous → the existing safe default. A spoofed bare claim (no signed
`flint_kind`, no real `act`) must not classify as Agent. Test both.

## Verification

`cargo check/clippy --workspace -- -D warnings` + `cargo test --workspace`;
≥80% new-code coverage; a spoof-resistance test + an `act`→Agent test.
