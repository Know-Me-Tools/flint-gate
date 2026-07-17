# written_by in Policy List Spec

## ADDED Requirements

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
