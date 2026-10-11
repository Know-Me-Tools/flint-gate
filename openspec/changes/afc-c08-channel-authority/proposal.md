# C08 channel authority provider

## Problem

The C02 governed-effect request accepts `GrantRef.active` and `GrantRef.revision`
from its authenticated execution-owner caller. That contract does not make Gate
the authoritative source of a channel disclosure or delivery grant, and it has
no source occurrence or route identity. A queued channel copy can therefore be
released after a caller's grant assertion has become stale.

## Change

Add `afc.channel-authority/1` as an additive private admin API. Gate persists
issuer-scoped channel grants and exact effect receipts in Postgres. It treats
source disclosure, recipient delivery, selected handler execution, scoped
reply, and route reassignment as five distinct Cedar actions. The first evaluation creates a durable
eligibility receipt; queue release requires a separate request that rechecks
the current Gate-owned grant revision and Cedar snapshot. Gate returns the
current revisions and a separate release receipt. It never routes, queues,
posts, or executes a channel effect.

## Boundary

The provider is available only with Gate's Postgres database and the existing
pinned-issuer authenticated admin boundary. The previous
`afc.governed-effect/1` API and approval lifecycle stay unchanged. The consumer
must pin this contract and call `/authority/channels/release` immediately
before crossing its effect boundary; a prior `eligible` response is not a
permit to release queued work.
