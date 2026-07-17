# add-policy-version-diff-view

## Summary

Add a "Diff" mode to the version history panel's view pane. When the operator selects a version, they can toggle between the plain text view (current behavior) and a unified diff view comparing the selected version against its immediately prior version (N vs. N-1). Uses the `diff` npm package (~15 KB gzipped) for pure-JS diff computation.

## Why

The "View" read-only pane shows a version's full text but gives no signal about what changed between versions. The diff view makes rollback intent observable — the operator can see the delta before confirming a rollback.

## What Changes

### New dependency

- `web/package.json`: add `"diff": "^7.0.0"` to dependencies.
- (No new devDependency — `diff` is used at runtime.)

### Frontend

- `web/src/pages/Policies.tsx` — `PolicyVersionHistory`:
  - Add a `viewMode: 'text' | 'diff'` toggle button next to the version view pane header (only shown when a version is selected and a prior version exists).
  - In `'diff'` mode: find the version with `version_num = viewedVersion.version_num - 1` from `history.versions`; if found, compute `diff.createPatch(policyId, priorText, selectedText, 'prior', 'selected')` and render the unified diff as colored lines: lines starting with `+` in green, `-` in red, `@` in muted foreground. If no prior version is available (N=1 or prior not loaded), show "No prior version to compare — this is the first version."
  - In `'text'` mode: current read-only `<Textarea>` behavior unchanged.
  - Reset `viewMode` to `'text'` when `viewedVersion` changes.
