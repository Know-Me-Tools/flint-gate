# Design: governed-effect authority v1

## Contract

`GovernedEffectRequest` is the immutable authority input. It carries the
effect and invocation identities, a versioned qualified action, canonical
resource destination, canonical SHA-256 payload digest, verified identity
chain, issuer-scoped grants, lease and budget references, and
runtime/host/catalog epochs. Canonical JSON serialization of that request is
hashed to produce the request binding. The caller cannot supply policy
authority: Gate resolves and hashes the active Cedar snapshot itself.

Lease and budget values are typed execution-owner facts. The private transport
accepts them only when the request authenticated through a pinned-issuer admin
provider representing the trusted P1 host/UAR boundary. Gate adds the fact
source, attestation issuer, and attestation subject to the decision binding.
API keys, anonymous callers, missing issuer pins, and unauthenticated loopback
requests cannot attest these facts. Both facts carry active state, revision,
and expiry, and Gate denies an inactive or expired lease or budget.

The provider validates the protocol and structural identity invariants before
Cedar. Missing or invalid authority facts deny visibly. Cedar receives the
full request as context and returns one of three outcomes. Each outcome binds
Gate's current content-addressed policy revision and digest:

- `deny`: no effect may run;
- `challenge`: a durable issuer-scoped challenge must be resolved and then
  revalidated;
- `permit`: the caller may execute only the bound request while its authority
  snapshot remains current.

## Approval lifecycle

A challenge record stores the exact request, binding, authority snapshot,
expiry, and decision. Decision commands are accepted only with an
authenticated administrator identity and persist both administrator subject
and issuer. Revalidation loads the issuer-scoped record and rejects absent,
expired, denied, stale, or differently bound challenges. It then reevaluates
Cedar and requires all policy, grant, lease, budget, identity, and epoch facts
to match the approved snapshot.

Memory and Postgres implement the same store contract. The Postgres table uses
`(issuer, challenge_id)` as its primary key, so equal challenge IDs from
different trust domains never collide. The configured Postgres backend is the
actual provider store; startup fails if it cannot be constructed.

## Transport

The private admin API is an adapter over the provider:

- `POST /authority/effects/evaluate`
- `POST /authority/effects/revalidate`
- `POST /authority/effects/{issuer}/{challenge_id}/decision`

The adapter authenticates the P1 execution owner and decision authors through
the existing pinned-issuer admin middleware. It serializes provider outcomes
but contains no execution logic.
