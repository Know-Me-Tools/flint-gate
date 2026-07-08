# config-governance

## MODIFIED Requirements

### Requirement: Startup agent-governance lint covers all served routes
The gateway SHALL lint the **merged** route set actually served — YAML routes plus
any database-sourced routes merged under `database.override_yaml` — not the YAML
route set alone, so `strict_agent_governance` is a comprehensive guarantee rather
than a boot-YAML-scoped one.

#### Scenario: A DB-only under-governed agent route is flagged at startup
- **WHEN** an agent-reachable route exists only in the database (not YAML) with a
  non-agent-scoped budget, no authorize hook, or a lifetime agent budget, and
  `database.override_yaml` is enabled
- **THEN** the startup lint emits a finding for it, and under
  `strict_agent_governance` the gateway refuses to start

#### Scenario: YAML routes are not double-reported
- **WHEN** the merged set is linted at startup
- **THEN** a route present in both YAML and the merged set is reported at most once

## ADDED Requirements

### Requirement: Hot-reload governance lint is non-terminating
The gateway SHALL re-lint the merged route set on every route hot-reload and
surface findings without terminating the running process, because a live process
cannot refuse-to-start.

#### Scenario: Reload under strict mode rejects and retains
- **WHEN** a route hot-reload (a `routes` change) would introduce an
  under-governed agent route and `strict_agent_governance` is set
- **THEN** the offending route is not applied, the last-good router is retained,
  and the rejection is logged — the process keeps running

#### Scenario: Reload without strict mode warns and applies
- **WHEN** the same reload occurs without `strict_agent_governance`
- **THEN** the finding is logged as a warning and the reloaded route set is still
  applied (advisory, non-breaking)

#### Scenario: A rejected reload is observable
- **WHEN** a route hot-reload is rejected under strict mode
- **THEN** a metric counter is incremented so the rejection is alertable, not only
  discoverable by log inspection

