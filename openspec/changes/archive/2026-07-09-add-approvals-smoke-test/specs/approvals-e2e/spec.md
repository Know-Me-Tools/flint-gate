## ADDED Requirements

### Requirement: The Approvals page SHALL render a "Pending Approvals" heading and an empty-state message when the API returns an empty list

`web/e2e/approvals.spec.ts` MUST confirm that navigating to `/approvals` with an empty
`GET /api/approvals` response shows both the page heading and the empty-state copy,
using `page.route()` interceptors so no live admin server is required.

#### Scenario: Empty approvals state renders heading

Given `/api/approvals` returns `{ approvals: [] }`,
when the operator navigates to `/approvals`,
then a heading with text "Pending Approvals" SHALL be visible on the page.

#### Scenario: Empty approvals state renders empty-state message

Given `/api/approvals` returns `{ approvals: [] }`,
when the operator navigates to `/approvals`,
then the text "No pending approvals." SHALL be visible on the page.

### Requirement: The Approvals page SHALL render a table with an "Approval ID" column header and approval row data when the API returns one or more approvals

`web/e2e/approvals.spec.ts` MUST confirm that a non-empty approval list renders as a
table with the expected column headers and that the fixture `approval_id` value is
visible in the table body.

#### Scenario: Table renders "Approval ID" column header

Given `/api/approvals` returns one approval entry,
when the operator navigates to `/approvals`,
then a column header with text "Approval ID" SHALL be visible.

#### Scenario: Table renders the approval_id cell value

Given `/api/approvals` returns a fixture containing `approval_id: "appr-test-001"`,
when the operator navigates to `/approvals`,
then the text "appr-test-001" SHALL be visible in the table.

#### Scenario: Table renders principal_id and action fields

Given `/api/approvals` returns a fixture with `principal_id: "agent:researcher"` and `action: "call_tool"`,
when the operator navigates to `/approvals`,
then both "agent:researcher" and "call_tool" SHALL be visible in the table.

### Requirement: The Approvals page SHALL re-fetch `/api/approvals` automatically on a 5-second interval without any user interaction

`web/e2e/approvals.spec.ts` MUST use `page.clock.install()` and `page.clock.tick()` to
advance fake time and verify the polling `setInterval` fires additional API calls without
waiting real seconds. The test MUST NOT rely on real-time delays.

#### Scenario: Second API call fires after one 5-second tick

Given `page.clock.install()` is called before navigation and a call counter is wired to
`/api/approvals`,
when the test navigates to `/approvals` and then calls `page.clock.tick(5_001)`,
then the call counter SHALL exceed its value immediately after mount.

#### Scenario: At least three API calls fire after two 5-second ticks

Given the same setup as above,
when the test calls `page.clock.tick(5_001)` twice after navigation,
then the call counter SHALL be at least 3 (initial mount fetch + 2 interval ticks).

### Requirement: The Approvals page SHALL be reachable from the root navigation link labelled "Approvals"

`web/e2e/approvals.spec.ts` MUST verify end-to-end navigation by starting at `/` and
clicking the "Approvals" nav link, confirming the page heading is visible after navigation.

#### Scenario: Navigation link exists and routes to the Approvals page

Given the test navigates to `/`,
when the operator clicks the link labelled "Approvals" in the navigation,
then a heading with text "Pending Approvals" SHALL be visible after navigation completes.
