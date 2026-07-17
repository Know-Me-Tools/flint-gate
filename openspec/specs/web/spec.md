# web Specification

## Purpose
TBD - created by archiving change add-approvals-auto-polling. Update Purpose after archive.
## Requirements
### Requirement: Approvals page polls for updates every 5 seconds

The Approvals page MUST automatically refresh the pending-approvals list every 5 seconds without manual user interaction.

#### Scenario: Page auto-refreshes at 5-second intervals
Given the operator has the Approvals page open
When 5 seconds elapse
Then the approvals list refetches silently in the background
And new or expired approvals appear without a page reload

#### Scenario: Polling stops when component unmounts
Given the operator navigates away from the Approvals page
When the component unmounts
Then the polling interval is cleared
And no further refetch calls are made

### Requirement: Policy list includes author attribution

The admin policy list MUST surface who last modified each policy without requiring the operator to open the history panel.

#### Scenario: Written-by appears in the policy table
Given the operator navigates to the Policies page
And at least one policy has been created or updated via the admin API with a JWT-authenticated request
When the policy list loads
Then the "Last by" column displays the JWT subject of the user who last edited the policy

#### Scenario: Policies without a version history show a dash
Given a policy exists that was created before version tracking was introduced
When the operator views the Policies page
Then the "Last by" column displays "—" for that policy

#### Scenario: JWT sub is captured on upsert
Given an authenticated admin operator sends a PUT /policies/{id} request
When the request carries a valid JWT with a sub claim
Then the sub value is stored as written_by on the new cedar_policy_versions row
And the Policies table reflects it immediately on next load

### Requirement: Version history panel supports paginated loading

The version history panel MUST allow operators to load additional version rows incrementally when a policy has more than 20 versions, rather than fetching all versions at once.

#### Scenario: Load more button appears when more versions exist
Given the operator opens the version history panel for a policy with more than 20 versions
When the initial 20 versions load
Then a "Load more" button appears below the version table

#### Scenario: Load more appends next page of versions
Given the "Load more" button is visible
When the operator clicks it
Then the next 20 versions are fetched and appended to the existing list
And the offset advances by 20

#### Scenario: Load more button disappears when all versions are loaded
Given the operator has clicked "Load more" enough times to exhaust all versions
When the last page returns fewer than 20 rows
Then the "Load more" button disappears

#### Scenario: Spinner shows while loading next page
Given the operator clicks "Load more"
When the fetch is in progress
Then a spinner appears inside the "Load more" button and the button is disabled

### Requirement: Version history panel shows unified diff between adjacent versions

The version history panel MUST allow operators to view a colored unified diff between any selected version and its immediate predecessor, making rollback intent observable before confirming.

#### Scenario: Diff toggle appears for versions with a predecessor
Given the operator opens the version history panel and clicks "View" on a version with version_num > 1
When the version pane renders
Then a "Text / Diff" toggle is visible in the pane header

#### Scenario: Diff toggle is hidden for version 1
Given the operator clicks "View" on version 1 (the first version)
When the version pane renders
Then no "Text / Diff" toggle is shown (there is no predecessor to diff against)

#### Scenario: Diff mode renders a colored unified patch
Given the operator selects "Diff" in the toggle
When the prior version is present in the loaded history
Then a unified diff is rendered with added lines in green, removed lines in red, and hunk headers in muted color

#### Scenario: Diff mode handles missing prior version gracefully
Given the operator selects "Diff" for a version whose predecessor has not been loaded yet
When the prior version is absent from history.versions
Then the pane displays "No prior version available in this page — load more to compare"

#### Scenario: View mode resets to Text when a different version is selected
Given the operator is viewing a diff
When the operator clicks "View" on a different version row
Then the pane switches back to Text mode

