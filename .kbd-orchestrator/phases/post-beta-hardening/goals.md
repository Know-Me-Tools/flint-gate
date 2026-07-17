# Goals — post-beta-hardening

_Seeded from: beta-release-readiness/reflection.md (2026-07-10)_

The beta-release-readiness phase closed all 14 identified gaps and left
flint-gate defensible as an external beta artifact. This phase addresses the
known limitations carried forward and raises the bar from "beta-defensible" to
"production-ready for sustained use."

## Goals

1. **Eliminate the cross-replica approval band-aid** — replace the
   `sessionAffinity: ClientIP` sticky-session workaround with a
   Postgres-backed shared approval store so approval decisions are
   correct regardless of pod restarts, evictions, or load balancer
   behavior. This is the highest-priority remaining correctness gap.

2. **Wire integration tests into CI** — the approval TTL expiry test and
   other `docker-compose`-dependent integration tests must run on every
   PR automatically, not just locally. A broken expiry path should fail
   CI, not slip into production undetected.

3. **Admin UI Cedar policy editor** — operators currently must use the
   REST API directly to create, edit, and delete Cedar policies. A policy
   editor in the admin UI with inline syntax validation and a preview of
   the authorization impact would significantly reduce the operator
   burden and the risk of policy typos.

4. **Metrics and observability documentation** — `metrics.rs` exposes
   Prometheus counters and histograms that operators need to monitor
   in production. A `metrics.md` reference page and a sample Grafana
   dashboard definition would lower the operational barrier for new
   deployments.

5. **Agent SDK enhancements** — the Go and TypeScript SDKs are functional
   but minimal. Token refresh on 401, structured retry-on-429, typed
   error responses, and streaming utility helpers would make them
   production-ready for the agent framework integrations beta customers
   will build.
