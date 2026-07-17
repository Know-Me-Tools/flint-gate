# Change: add-policies-ui-smoke-tests

## Summary

New Playwright spec `web/e2e/policies.spec.ts` covering the three Policies page features shipped
in `admin-ux-polish-and-diff-view`: "Last by" column, "Load more" pagination, and the
"Text / Diff" diff toggle.

## Motivation

Zero Playwright coverage exists for the Policies page. Three features shipped without E2E tests:
the `written_by` column, history pagination, and the `PolicyDiffView` component. If any of these
regressions silently, CI has no signal.

## Design

All tests use `page.route()` interceptors — no real admin server required. Fixture JSON is
inlined in the spec.

### Fixture data

```
policies: [
  { id: 'pol-alpha', policy_text: 'permit(…);', enabled: true, written_by: 'alice' },
  { id: 'pol-beta',  policy_text: 'forbid(…);', enabled: true, written_by: null },
]

history(pol-alpha, page 1): { policy_id: 'pol-alpha', total_hint: 25, versions: [v20…v1] }
history(pol-alpha, page 2): { policy_id: 'pol-alpha', total_hint: 25, versions: [v25…v21] }
```

### Test cases

1. **"Last by" column renders** — mock `/api/policies`, navigate to `/policies`, assert
   `<th>Last by</th>` visible; assert `alice` in row 1; assert `—` in row 2.

2. **"Load more" appears and appends rows** — mock `/api/policies` + `/api/policies/pol-alpha/history`
   returning `total_hint: 25, versions: [20 rows]`; open history panel; assert 20 rows; assert
   "Load more" button visible; mock second page (5 rows); click "Load more"; assert 25 rows total;
   assert "Load more" gone.

3. **"Diff" toggle hidden for v1, shown for v2+** — mock history with two versions (v1 + v2);
   click v1 row → assert toggle absent; click v2 row → assert "Text" and "Diff" buttons visible.

4. **"Diff" view renders colored lines** — with v2 selected, click "Diff"; assert `<pre>` visible;
   assert at least one `+` line is present (any line starting with `+` that is not `+++`).

## Affected Files

- `web/e2e/policies.spec.ts` (new)

## Verification

`pnpm --dir web test:e2e --grep "Policies"` exits 0.
