# add-history-panel-pagination

## Summary

Add "Load more" pagination to the version history panel in the policy editor. The backend `GET /policies/{id}/history` already supports `?offset=N&limit=N`; the UI always fetches page 0 and has no controls for subsequent pages.

## Why

Policies with frequent writes accumulate many versions. The current default limit is 20; without pagination the operator can only see the 20 most recent versions.

## What Changes

- `web/src/pages/Policies.tsx` — `PolicyVersionHistory` component:
  - Add `offset: number` and `hasMore: boolean` to component state (initial: `offset=0`, `hasMore=false`).
  - On initial load, set `hasMore = versions.length >= 20` (heuristic; `total_hint` when non-null provides a more accurate signal).
  - Render a "Load more" button below the version table when `hasMore`.
  - On "Load more" click, call `fetchPolicyHistory(policyId, offset + 20, 20)` and append results; update `offset`; update `hasMore`.
  - Show a loading spinner inside the "Load more" button while fetching.
