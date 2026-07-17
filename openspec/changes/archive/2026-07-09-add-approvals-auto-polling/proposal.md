# add-approvals-auto-polling

## Summary

Restore ≤5-second auto-refresh on the Approvals page. The prior TanStack Query `refetchInterval` option was removed during migration to `@prometheus-ags/prometheus-entity-management`, which does not expose polling in `ListQueryOptions`. The `useApprovals().refetch` handle is available but never called on a timer — operators must manually navigate away and back to see new approvals.

## Why

Approvals have a timeout (`expires_at`). An operator who navigates to the Approvals page must see new requests within a few seconds, not only on page load. A 5-second poll matches the prior behavior.

## What Changes

- `web/src/pages/Approvals.tsx` — add a `useEffect` that reads `refetch` from `useApprovals()` and calls it on a 5 000 ms `setInterval`; clear the interval on unmount.
- No backend changes.
- No new dependencies.
