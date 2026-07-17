# Policy Version Diff View Spec

## ADDED Requirements

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
