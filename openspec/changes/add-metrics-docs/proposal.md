# Proposal — add-metrics-docs

**Phase:** post-beta-hardening
**Goal:** G-4 — Metrics and observability documentation
**Severity:** MEDIUM
**Depends on:** —

## Problem

`metrics.rs` exposes 6 named Prometheus metrics on `GET /metrics` (admin port).
No reference documentation exists. Operators deploying flint-gate have no
canonical list of metric names, label sets, or alert thresholds.

## Scope

- `docs/docs/metrics.md` — new reference page
- `grafana/flint-gate-dashboard.json` — sample Grafana 10 dashboard
- `docs/sidebars.ts` — add metrics.md under Operations category
- `docs/docs/operations.md` — add `GET /metrics` reference and link

## Out of scope

- Changes to `metrics.rs` (implementation is already correct)
- Alertmanager rules (operator-specific)
- Grafana provisioning YAML (dashboard JSON is sufficient for import)

## Acceptance Criteria

- `docs/docs/metrics.md` exists and documents all 6 metrics:
  `flint_delegate_total`, `flint_local_exchange_total`,
  `flint_delegate_latency_seconds`, `flint_tool_authz_total`,
  `flint_agent_budget_denied_total`,
  `flint_governance_reload_rejected_total`
- Each metric entry includes: name, type, label set, description,
  and a suggested alert threshold
- `grafana/flint-gate-dashboard.json` is valid JSON
- Sidebar entry for `metrics` renders under Operations
- No existing tests break
