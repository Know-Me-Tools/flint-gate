# admin-ui Specification

## Purpose
TBD - created by archiving change add-policy-version-history-ui. Update Purpose after archive.
## Requirements
### Requirement: Policy version history panel
The admin UI SHALL display a collapsible "Version History" panel on the policy editor that lazy-loads prior versions from `GET /policies/{id}/history` when expanded, renders each version's `version_num`, `written_at`, and `written_by`, and supports "View" and "Rollback" actions.

#### Scenario: Version history lazy-loads on expand
- **WHEN** the operator expands the "Version History" panel for a policy
- **THEN** the UI requests `GET /policies/{id}/history` and renders the list of version rows ordered by version number descending

#### Scenario: Version history shows loading state
- **WHEN** the history fetch is in flight
- **THEN** a loading spinner is shown inside the panel until the data arrives

#### Scenario: Version history shows error state
- **WHEN** the history fetch returns a non-2xx status
- **THEN** an inline error message is displayed inside the panel with the error detail

#### Scenario: Viewing a prior version populates read-only pane
- **WHEN** the operator clicks "View" on a version row
- **THEN** the version's `policy_text` is displayed in a read-only textarea adjacent to the version list

#### Scenario: Restore to editor copies text without saving
- **WHEN** the operator clicks "Restore to editor" in the version view pane
- **THEN** the editable policy textarea is populated with the selected version's `policy_text` and no save/upsert call is made

#### Scenario: Rollback requires explicit confirmation
- **WHEN** the operator clicks "Rollback" on a version row
- **THEN** a confirmation dialog appears before any API call is dispatched

#### Scenario: Confirmed rollback calls rollback endpoint and refreshes list
- **WHEN** the operator confirms the rollback dialog
- **THEN** `POST /policies/{id}/rollback` is called with the selected `version_num`, and on success the policy list query is invalidated and the history panel refreshes

#### Scenario: Rollback error displays inline
- **WHEN** `POST /policies/{id}/rollback` returns a non-2xx status or a 422 Cedar parse error
- **THEN** the error (including Cedar policy parse messages when present) is displayed inline in the history panel without closing the panel

#### Scenario: Admin UI uses prometheus-entity-management store
- **WHEN** the admin UI fetches or mutates policy data
- **THEN** it uses `@prometheus-ags/prometheus-entity-management` hooks (`useEntityList`, `useEntityMutation`) rather than `@tanstack/react-query` directly, and the global entity graph store provides list invalidation via `useGraphStore.getState().setListStale`

