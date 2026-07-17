# sdk Specification

## Purpose
TBD - created by archiving change add-sdk-policy-methods. Update Purpose after archive.
## Requirements
### Requirement: FlintGateAdmin exposes full Cedar policy CRUD and history surface

The `FlintGateAdmin` TypeScript SDK class MUST provide typed methods for every Cedar policy admin endpoint so external callers do not hand-roll fetch calls.

#### Scenario: listPolicies returns typed policy rows
Given a caller has a configured FlintGateAdmin instance
When they call `admin.listPolicies()`
Then the method issues GET /policies and returns a `PolicyRow[]`

#### Scenario: getPolicy fetches a single policy by id
Given a valid policy id
When `admin.getPolicy(id)` is called
Then the method issues GET /policies/{id} and returns a `PolicyRow`

#### Scenario: createPolicy POSTs a new Cedar policy
Given a `UpsertPolicyInput` with a valid id and policy_text
When `admin.createPolicy(input)` is called
Then the method issues POST /policies with JSON body and returns `UpsertPolicyResponse`

#### Scenario: updatePolicy PUTs an updated Cedar policy
Given an existing policy id and a `UpsertPolicyInput`
When `admin.updatePolicy(id, input)` is called
Then the method issues PUT /policies/{id} with JSON body

#### Scenario: deletePolicy removes a policy
Given an existing policy id
When `admin.deletePolicy(id)` is called
Then the method issues DELETE /policies/{id}

#### Scenario: getPolicyHistory supports offset pagination
Given a policy id and opts with offset and limit
When `admin.getPolicyHistory(id, { offset: 20, limit: 10 })` is called
Then the query string includes offset=20 and limit=10

#### Scenario: rollbackPolicy writes version_num to the request body
Given a policy id and a target version number
When `admin.rollbackPolicy(id, 3)` is called
Then POST /policies/{id}/rollback is issued with body `{ "version_num": 3 }`

