# fix-multi-replica-rate-limit-warning

**Phase:** beta-release-readiness / Phase 3 (Serious gap S-9)

## Problem

When `rate_limiting.enabled: true` is configured without a Redis backend and
the operator runs more than one replica, each pod maintains an independent
in-memory rate-limit counter. Pod A's counter has no knowledge of Pod B's
requests; an agent can exhaust limits against two pods and receive 2× the
configured request budget. The current code has no warning for this
misconfiguration.

## Solution

At startup, when rate limiting is enabled AND no Redis connection string is
configured AND `replicas > 1` is detectable, emit a structured `WARN` log
explaining the split-counter risk and recommended remediation (configure Redis
or set `replicas: 1`).

Detection logic:
1. Read `config.rate_limiting.enabled` and `config.rate_limiting.redis_url`
2. Read `KUBERNETES_SERVICE_HOST` env var (set in any K8s pod) as a proxy for
   "am I running in K8s?"
3. If rate limiting is enabled, redis_url is None, and we are in K8s → emit
   warning

The warning should be `WARN` (not fatal) because in-memory rate limiting is
still better than none, and the operator may be running a single pod.

## Files to change

- `crates/flint-gate-core/src/main.rs` — add startup guard after config is
  loaded, before the server starts accepting connections
- `config.example.yaml` — add comment documenting the Redis requirement for
  multi-replica deployments
