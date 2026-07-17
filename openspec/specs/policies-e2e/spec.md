# policies-e2e Specification

## Purpose
TBD - created by archiving change add-policies-ui-smoke-tests. Update Purpose after archive.
## Requirements
### Requirement: The Policies page SHALL have Playwright smoke tests covering the three new UI surfaces

`web/e2e/policies.spec.ts` MUST provide deterministic E2E coverage for the
"Last by" column, history panel pagination, and version diff toggle using
`page.route()` interceptors (no live admin server required).

#### Scenario: "Last by" column header renders

Given the admin API returns two policies (one with `written_by: 'alice'`, one with `written_by: null`),
when the operator navigates to `/policies`,
then the "Last by" column header SHALL be visible, `alice` SHALL appear in the first row,
and `—` SHALL appear in the second row.

#### Scenario: "Load more" button appears when more versions exist

Given the history API returns 20 versions with `total_hint: 25`,
when the operator opens the Version History panel for a policy,
then 20 version rows SHALL be visible and a "Load more" button SHALL be present.

#### Scenario: "Load more" appends rows and disappears when all loaded

Given the "Load more" button is visible,
when the operator clicks it and the second page (5 versions) is returned,
then 25 total version rows SHALL be visible and the "Load more" button SHALL no longer be visible.

#### Scenario: "Text / Diff" toggle hidden for v1, visible for v2+

Given a policy with two versions in history,
when the operator views v1,
then the "Text / Diff" toggle SHALL NOT be visible;
when the operator views v2,
then "Text" and "Diff" buttons SHALL both be visible.

#### Scenario: Diff view renders unified diff with addition lines

Given v2 is selected and the "Diff" mode button is clicked,
then a `<pre>` element SHALL be visible containing at least one line starting with `+`
(but not `+++`) indicating an added line in the diff output.

