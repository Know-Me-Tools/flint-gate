# Agent Fabric Convergence: Flint Gate planning notes

Date: 2026-09-25

Status: planning input; no schema or runtime contract is ratified here

Source baseline: `0edb945f1d73bd8567b90ab6640fbbd0cf1a00c0`

## Role in the convergence

Flint Gate is the policy enforcement and delegated-identity boundary for protected effects. The convergence plan's C05 keeps the BossFang workflow while delegating a complete run to UAR, and C06/C09 place runtime instances and team state in UAR. C01 must still reconcile those statements with the accepted P1 contracts. On that dependency basis, Gate must not become either system's scheduler, workflow engine, or event transport.

At an effect boundary, UAR or another authorized caller should present verified subject and actor identity plus the exact action, resource, request payload identity, and applicable policy/grant revisions. Gate should decide allow, deny, or require approval against current Cedar policy and current revocation state before the effect occurs. A role or agent definition can describe requested capabilities; it cannot grant them.

This division also applies to Codex CLI execution. Codex remains a Rust-hosted execution option behind UAR's admitted run and effect path. A Codex role, prompt, skill, handoff, or local process identity must not bypass Gate or turn into authority by being named in an agent document.

## Current source facts

- The Cedar schema currently defines `User`, `Agent`, `Service`, and `Route`, with one `call_tool` action (`crates/flint-gate-core/src/authz/schema.rs`). This is a real tool boundary, but not yet the full action/resource vocabulary required by C02.
- `AuthzEngine` uses immutable bundle snapshots and fail-closed decisions; request code can pin a snapshot so route and policy revisions do not mix during long work (`crates/flint-gate-core/src/authz/engine.rs`).
- `ToolAuthzContext` carries principal kind, principal id, route id, and revocation state. Tool arguments enter Cedar context, while deny audit intentionally omits arguments (`crates/flint-gate-core/src/authz/tool_authz.rs`).
- RFC 8693 exchange downscopes to the subject's scopes and rejects `actor_token`; arbitrary multi-hop delegation is therefore explicitly unsupported at this baseline (`crates/flint-gate-core/src/auth/token_exchange.rs`).
- Delegated token identity uses a validated `act.sub`; externally supplied `flint_kind` is stripped to prevent principal-kind escalation (`crates/flint-gate-core/src/auth/identity.rs`).
- Postgres can persist pending approvals across replicas, but the current row contains agent, tool, reason, expiry, and decision fields rather than an exact issuer/policy/grant/payload binding (`crates/flint-gate-core/migrations/0003_pending_approvals.sql`).
- The ASO authority path already models deployment/incarnation/revision high-water marks and issuer-scoped denied sessions (`crates/flint-gate-core/src/authority/mod.rs`, migration `0004_aso_authority_cursors.sql`). C02 should consume that accepted authority contract rather than build a parallel revocation cache.

## Required boundary for C02 and later changes

The next Gate work should extend the existing admission path rather than add a second policy engine. C01 must first settle the shared identity/state/action vocabulary and record the accepted UAR P1 approval contract. C02 can then bind each protected effect and approval to the initiating subject, delegated actor, issuer/audience, action/resource, immutable payload identity, policy revision, grant revision, expiry, and decision state. Exact names and storage shape remain C01/C02 design decisions.

Approval is authorization evidence, not effect execution. After a wait, the effect owner must recheck current policy, revocation, payload identity, and any lease before committing the effect. A durable approval row alone must not make a stale action executable. Unknown effect outcomes remain the effect owner's reconciliation problem.

Later slices consume this boundary:

- C08 uses Gate to authorize source disclosure and recipient delivery/execution while Fabric only transports messages.
- C10 uses Gate for connector effects; an observed feedback item or drafted issue does not authorize publication or implementation.
- C17 uses separate representation and organizational grants. A C-level title, simulation, or personal profile must never satisfy a human or organization approval check.

## Dependencies and next repository-scoped KBD child

Recommended next child: **C02 Flint Gate governed action and approval boundary**.

It should remain blocked until C01 records the accepted UAR P1 contract and the D-GATE source checkpoint. Its initial owned surface should be limited to the existing authz, approval, delegated-identity, authority-freshness, and policy-installation modules. Acceptance must exercise the real protected-effect path with forged identity, stale or revoked authority, changed payload, expired approval, and policy failure; a denied effect must not occur.

Do not include team scheduling, workflow state, Fabric delivery, connector implementation, or agent-definition parsing in this child.

## Evidence to preserve in the child plan

- Baseline commit and exact UAR/Gate dependency revisions.
- Current Cedar schema/action limits and migration compatibility.
- Existing authority high-water and denied-session contract.
- Existing token-exchange refusal of multi-hop delegation.
- Approval backend selected for each supported profile and its restart/replica semantics.
- One end-to-end receipt joining run, approval, policy/grant revision, and effect outcome without logging credentials or sensitive payloads.
- A kickoff recheck that the recorded Gate and UAR baselines, dependency graph, and single-owner assignments have not changed.
