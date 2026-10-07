# Proposal: AFC C02 governed-effect authority v1

## Problem

Flint Gate can evaluate Cedar policy and can persist approval rows, but those
capabilities are not one authority path. Live stream approvals use an
in-process routing table, while the selected durable store is constructed and
not used by the live decision surface. A caller also cannot bind a decision to
an exact effect, payload, identity, policy, grant, lease, budget, or runtime
epoch. A decision can therefore outlive the authority facts it was based on.

## Change

- Define transport-neutral protocol `afc.governed-effect/1`.
- Evaluate an exact governed-effect request through the active Cedar engine.
- Resolve the active Cedar policy set inside Gate and bind its content-addressed
  set ID, revision, and digest. Client policy fields are not authority.
- Accept lease and budget facts only under the authenticated P1 execution
  owner's issuer and subject, recording that fact-source attestation.
- Return `deny`, `challenge`, or `permit` with a canonical request binding and
  the authority revisions used for the decision.
- Persist approval challenges under the issuer-scoped identity
  `(issuer, challenge_id)` in memory or Postgres.
- Record the authenticated administrator that resolves a challenge.
- Require revalidation after a wait and deny when the request binding,
  identity, policy, grant, approval, lease, budget, or epoch changed.
- Expose the provider through the private authenticated admin surface. Gate
  remains a policy and approval authority and never executes the effect.
- Refuse startup when `approval.backend=postgres` is selected without a
  database instead of silently weakening durability to memory.

## Boundaries

The caller remains responsible for executing an effect after a permit. This
change does not add a scheduler, executor, queue, local bypass, or policy
fallback. Existing streaming protocols keep their current wire format.
