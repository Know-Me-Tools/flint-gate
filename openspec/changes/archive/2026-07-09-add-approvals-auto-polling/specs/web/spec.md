# Approvals Auto-Polling Spec

## ADDED Requirements

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
