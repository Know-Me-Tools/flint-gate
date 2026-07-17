---
type: Reference
id: surreal-memory-server-b2ed891-rebuild-and-launchd-restart-plan
title: surreal-memory-server b2ed891 rebuild and launchd restart plan
tags:
- surreal-memory-server
- surrealdb
- launchd
- deployment
- health-check
- arm64-signing
sources:
- stdin
timestamp: 2026-07-03T13:23:22.248763+00:00
created_at: 2026-07-03T13:23:22.248763+00:00
updated_at: 2026-07-03T13:23:22.248763+00:00
revision: 0
---

## Session Context

- **Project:** `surreal-memory-server`
- **Phase:** `surrealdb-connection-architecture`
- **KBD root:** `/Users/gqadonis/Projects/prometheus/prometheus-skill-pack/tools/surreal-memory-server`
- **Captured:** `2026-07-03T13:19:55Z`
- **Source record:** `manual:surreal-memory-server/surrealdb-connection-architecture`

## Current Service State

- `surreal-memory-server` is running on port `23001`.
- `/health` returns HTTP `200`.
- The service is currently serving from the old binary to maintain zero downtime during rebuild.
- The machine is otherwise idle; the rebuild is a single focused build with no competing workload.

## Build State

- Correct source revision: `b2ed891`.
- Build of the `b2ed891` code is in progress.
- The currently running binary was confirmed stale: `b2ed891` sources are newer than the built artifact.

## Deployment Plan

When the build completes:

1. Deploy the freshly built `surreal-memory` binary to:
   - `~/.local/bin`
   - `/usr/local/bin`
2. Ad-hoc re-sign the arm64 binary.
3. Clean-restart the `launchd` service:
   - kill the wedged process if present
   - `kickstart` the service
4. Verify `/health` reliably returns HTTP `200` from the new binary.

## Immediate Next Action

Swap in the freshly built `b2ed891` `surreal-memory` binary, restart the service, and verify the health endpoint is reliably green on current code.

# Citations

1. stdin