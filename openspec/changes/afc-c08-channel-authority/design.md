# C08 channel authority design

Gate stores `(issuer, grant_id)` with a monotonically increasing revision,
active state, exact action/scope/recipient/handler/classification, and causal
limits. An authenticated administrator with the matching pinned issuer creates
or changes a grant using a revision comparison and revokes it with the same
comparison. No client chooses its revision or active state. Postgres row locks
serialize grant mutation with effect evaluation and release.
The sender filter is either one exact sender or an administrator-authored `*`
within that exact provider/account/workspace/room/thread; no other scope field
supports a wildcard.

An effect request names a stable occurrence, separate actor identity and
authenticated execution owner, exact provider/account/workspace/room/thread/
sender scope, selected recipient and handler, route revision, SHA-256 payload
digest, classification, and inherited causal budget. Gate hashes the canonical
request together with the authenticated owner's attestation. Distinct actions
have distinct Cedar action IDs and receipt identities. Scope mismatch, exhausted
causal budget, revoked grant, or Cedar denial withholds the effect.
`route_reassignment` is a fifth action. Its `route_revision` is the expected
current affinity revision, and its recipient/handler identify the proposed
destination. The Gate release receipt authorizes only that exact proposal;
BossFang must still compare-and-swap its own durable route row. An ordinary
message mention or generic handler-execution permit cannot change affinity.

`POST /authority/channels/evaluate` records `eligible` or `withheld` but cannot
release. `POST /authority/channels/release` requires the same effect identity
and original eligible evaluation; it locks the current grant, compares its
revision and the current Cedar digest to the evaluation, reevaluates Cedar, and
stores a second receipt. Duplicate release returns `uncertain`, since Gate
cannot know whether the caller executed the first release. A policy reload
during the release check withholds the result. The caller must reconcile any
uncertain external effect by its own stable action ID.

`GET /authority/channels/capabilities` reports the versioned contract and
whether Postgres-backed authority is available. Admin routes use the existing
pinned-issuer authentication middleware; API-key and anonymous credentials
cannot attest grants or execution-owner facts.

Gate does not become a scheduler, handler, transport cursor, or effect
executor. BossFang retains route selection and reply delivery; Fabric carries
the versioned envelope; UAR retains execution ownership.
